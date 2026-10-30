
impl<'ctx> HirCompiler<'ctx> {
    fn compile_json_backend_call(
        &mut self,
        args: &[HirExpr],
        backend_symbol: &str,
        source_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if backend_symbol.starts_with("thaw_js_") {
            self.uses_quickjs = true;
        }
        if backend_symbol.starts_with("thaw_napi_") {
            self.uses_napi = true;
        }
        let [name, call_args] = args else {
            return Err(format!(
                "{source_name} expects exactly two arguments (name, args)"
            ));
        };
        if backend_symbol.starts_with("thaw_js_") {
            return self.compile_with_owned_inline_dynamic_callee(
                name,
                "json_backend_callee_cleanup",
                |this, name_val| {
                    this.compile_json_backend_call_with_name(
                        name_val, call_args, backend_symbol,
                    )
                },
            );
        }
        let name_val = self.compile_expr(name)?;
        self.compile_json_backend_call_with_name(name_val, call_args, backend_symbol)
    }

    fn compile_json_backend_call_with_name(
        &mut self,
        name_val: BasicValueEnum<'ctx>,
        call_args: &HirExpr,
        backend_symbol: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // See the matching comment on `compile_call_dynamic_method`: a
        // `JsValue`-typed element nested in `call_args` (e.g. a
        // `registerNativeCallback` result) needs this flag set while it's
        // being marshaled, same as the method-call and typed-ambient-
        // declaration paths -- only meaningful for the QuickJS-NG backend,
        // N-API has no such reviver.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        if backend_symbol.starts_with("thaw_js_") {
            self.compiling_quickjs_dynamic_arguments = true;
        }
        let args_json_val = self.compile_expr(call_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let args_json_val = args_json_val?;

        self.compile_json_backend_values(name_val, args_json_val, backend_symbol)
    }

    fn compile_json_backend_values(
        &mut self,
        name_val: BasicValueEnum<'ctx>,
        args_json_val: BasicValueEnum<'ctx>,
        backend_symbol: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_values_with_extra(name_val, args_json_val, backend_symbol, &[], &[])
    }

    fn compile_json_backend_values_with_extra(
        &mut self,
        name_val: BasicValueEnum<'ctx>,
        args_json_val: BasicValueEnum<'ctx>,
        backend_symbol: &str,
        extra: &[BasicMetadataValueEnum<'ctx>],
        owned_handles: &[FreshEventKeyHandle<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let stringify_fn = self.module.get_function(
            if backend_symbol.ends_with("_graph_result") {
                "thaw_json_graph_encode"
            } else {
                "thaw_json_stringify"
            }
        ).unwrap();
        let args_json_str = self
            .builder
            .build_call(
                stringify_fn,
                &[args_json_val.into()],
                "call_dynamic_args_json",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_stringify did not return a value")?;
        let raw_owned_handles = HirCompiler::<'ctx>::fresh_event_key_raw_handles(owned_handles);
        let args_json_str = self.compile_check_json_stringify_error_with_inputs_and_handles(
            args_json_str, &[args_json_val], &[], &[], &raw_owned_handles,
        )?;
        self.release_fresh_event_key_handles(owned_handles)?;

        let call_fn = self.module.get_function(backend_symbol).unwrap();
        let mut call_args = vec![name_val.into(), args_json_str.into()];
        call_args.extend_from_slice(extra);
        let result = self
            .builder
            .build_call(
                call_fn,
                &call_args,
                "call_dynamic_result_abi",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{backend_symbol} did not return a value"))?
            .into_struct_value();
        let result_json_str = self
            .builder
            .build_extract_value(result, 0, "call_dynamic_value")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let error = self
            .builder
            .build_extract_value(result, 1, "call_dynamic_error")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        // A thaw-js error string becomes the framed `Error` (name, message, code, properties) it was.
        let error = if backend_symbol.starts_with("thaw_tjs_") {
            self.compile_thaw_js_frame_error(error)?
        } else {
            error
        };
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[args_json_str.into()],
                "destroy_call_dynamic_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[args_json_val.into()],
                "destroy_call_dynamic_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|e| e.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;

        let parsed = if backend_symbol.ends_with("_graph_result") {
            self.compile_decode_quickjs_graph(result_json_str.into())?
        } else {
            let parse_fn = self.module.get_function("thaw_json_parse").unwrap();
            let parsed = self.builder.build_call(parse_fn, &[result_json_str.into()], "call_dynamic_result")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or_else(|| "thaw_json_parse did not return a value".to_string())?;
            self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[result_json_str.into()], "destroy_call_dynamic_result_string")
                .map_err(|error| error.to_string())?;
            parsed
        };

        self.mark_fresh_owned_json_result(parsed)?;
        Ok(parsed)
    }

    fn compile_json_as_value(
        &mut self,
        json: BasicValueEnum<'ctx>,
        symbol: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.builder
            .build_call(
                self.module.get_function(symbol).unwrap(),
                &[json.into()],
                symbol,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{symbol} returned no value"))?;
        self.compile_check_json_host_error(value,
            matches!(symbol, "thaw_json_as_string" | "thaw_json_as_string_value").then_some("thaw_cstring_destroy"))
    }

    fn compile_json_as_bool_value(
        &mut self,
        json: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self
            .compile_json_as_value(json, "thaw_json_as_bool")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                value,
                self.context.i8_type().const_zero(),
                "dynamic_bool_result",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    #[allow(dead_code)]
    fn compile_native_object_to_json(
        &mut self,
        object: PointerValue<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_native_object_to_json_with_undefined(object, ty, false)
    }

    /// Emit one statically typed field after the caller selected it from the
    /// initial own-key snapshot. A live visibility check precedes every read.
    fn compile_native_object_json_field(
        &mut self,
        object: PointerValue<'ctx>,
        ty: &HirType,
        fields: &[(String, HirType)],
        index: usize,
        json: BasicValueEnum<'ctx>,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        let visibility_name = &fields[index].0;
        let stringify_hidden = if self.encoding_stringify_errors {
            self.stringify_hides_error_field(object, ty, fields, index)?
        } else {
            None
        };
        let hidden = if visibility_name.as_bytes().contains(&0) {
            self.context.bool_type().const_zero()
        } else {
            let marker_name = self.builder.build_global_string_ptr(
                visibility_name, "native_field_visibility_name",
            ).map_err(|error| error.to_string())?;
            self.builder.build_call(
                self.module.get_function("thaw_object_marker_hidden").unwrap(),
                &[object.into(), marker_name.as_pointer_value().into()],
                "native_field_hidden",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("native field visibility query returned no value")?.into_int_value()
        };
        let hidden = match stringify_hidden {
            Some(extra) => self.builder.build_or(hidden, extra, "stringify_hidden_error_field")
                .map_err(|error| error.to_string())?,
            None => hidden,
        };
        let function = self.current_function();
        let visibility_write = self.context.append_basic_block(function, "write_visible_native_field");
        let visibility_done = self.context.append_basic_block(function, "native_field_done");
        self.builder.build_conditional_branch(hidden, visibility_done, visibility_write)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(visibility_write);
        let (name, field_ty) = &fields[index];
        let getter = format!("__thaw_getter_{name}");
        let setter = format!("__thaw_setter_{name}");
        let getter = fields.iter().position(|(field, _)| field == &getter);
        let has_setter = fields.iter().any(|(field, _)| field == &setter);
        // console.log prints an accessor property as `[Getter]`/`[Setter]`/`[Getter/Setter]`
        // and never invokes it; JSON.stringify keeps calling it.
        let accessor_kind = match (getter.is_some(), has_setter) {
            _ if !self.console_omit_absent_fields => None,
            (true, true) => Some(3),
            (true, false) => Some(1),
            (false, true) => Some(2),
            (false, false) => None,
        };
        if let Some(kind) = accessor_kind {
            let sentinel = self.builder.build_call(
                self.module.get_function("thaw_json_accessor_sentinel").unwrap(),
                &[self.context.i8_type().const_int(kind, false).into()], "console_accessor_sentinel",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("accessor sentinel returned no value")?;
            let key = self.builder.build_global_string_ptr(name, "console_accessor_key")
                .map_err(|error| error.to_string())?;
            self.compile_json_object_set_native_with_undefined(
                json, key.as_pointer_value(), sentinel, &HirType::Json, preserve_undefined, false,
            )?;
        } else if getter.is_none() && has_setter {
            if preserve_undefined {
                let key = self.builder.build_global_string_ptr(name, "dynamic_setter_only_key")
                    .map_err(|error| error.to_string())?;
                let undefined = self.compile_napi_undefined_json()?;
                self.compile_json_object_set_native_with_undefined(
                    json, key.as_pointer_value(), undefined, &HirType::Json, true, true,
                )?;
            }
        } else {
            let (value, value_type) = if let Some(getter) = getter {
                self.compile_native_object_getter(object, fields, getter)?
            } else {
                (
                    self.compile_accessor_aware_field_read(object, ty, fields, index)?,
                    field_ty.clone(),
                )
            };
            let key = self.builder.build_global_string_ptr(name, "dynamic_object_key")
                .map_err(|error| error.to_string())?;
            // A class instance prints its absent optional fields as `undefined` (Node's
            // define semantics); plain objects and error-family instances omit them.
            let class_instance = fields.first().is_some_and(|(n, t)| {
                *t == HirType::Bool && n.strip_prefix("__thaw_class_identity_\u{1e}")
                    .is_some_and(|chain| !chain.split('\u{1f}').any(thaw_hir::is_error_family_name))
            });
            let before = std::mem::replace(&mut self.encoding_class_instance_fields, class_instance);
            let set = self.compile_json_object_set_native_with_undefined(
                json, key.as_pointer_value(), value, &value_type, preserve_undefined, false,
            );
            self.encoding_class_instance_fields = before;
            set?;
        }
        self.builder.build_unconditional_branch(visibility_done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(visibility_done);
        Ok(())
    }

    /// `JSON.stringify` of an Error-family record: `message` (and a built-in record's `cause`,
    /// `errors`, ...) are non-enumerable, and `name` is an own property only once assigned, which
    /// is approximated as "differs from the inherited family name". `None` when the field shows.
    fn stringify_hides_error_field(
        &mut self,
        object: PointerValue<'ctx>,
        ty: &HirType,
        fields: &[(String, HirType)],
        index: usize,
    ) -> Result<Option<inkwell::values::IntValue<'ctx>>, String> {
        let Some(chain) = fields.first().and_then(|(marker, _)| {
            marker.strip_prefix("__thaw_class_identity_\u{1e}")
        }) else {
            return Ok(None);
        };
        let Some(default) = chain.split('\u{1f}').find(|part| thaw_hir::is_error_family_name(part)) else {
            return Ok(None);
        };
        let builtin = chain.split('\u{1f}').next().is_some_and(thaw_hir::is_error_family_name);
        let (name, field_ty) = &fields[index];
        match name.as_str() {
            "message" => Ok(Some(self.context.bool_type().const_all_ones())),
            "cause" | "errors" | "error" | "suppressed" if builtin => {
                Ok(Some(self.context.bool_type().const_all_ones()))
            }
            "name" if *field_ty == HirType::Str => {
                let value = self.compile_accessor_aware_field_read(object, ty, fields, index)?;
                let default = self.builder.build_global_string_ptr(default, "stringify_default_error_name")
                    .map_err(|error| error.to_string())?;
                let compared = self.builder.build_call(
                    self.module.get_function("thaw_string_compare").unwrap(),
                    &[value.into(), default.as_pointer_value().into()],
                    "stringify_error_name_compare",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("string comparison returned no value")?.into_int_value();
                self.builder.build_int_compare(
                    IntPredicate::EQ, compared, self.context.i32_type().const_zero(), "stringify_name_is_default",
                ).map(Some).map_err(|error| error.to_string())
            }
            _ => Ok(None),
        }
    }

    /// A native `Map`/`Set` as the Json wrapper the inspector understands
    /// (`{ __thaw_map_entries__: [[k, v], ...] }` / `{ __thaw_set_values__: [...] }`).
    /// Only reachable from `console.log`, where it prints like Node (`Map(1) { 'a' => 1 }`).
    fn compile_native_collection_to_json(
        &mut self,
        collection: PointerValue<'ctx>,
        ty: &HirType,
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (snapshot, wrapper_key, element_type) = match ty {
            HirType::Array(element) => ("", "__thaw_array_items__", (**element).clone()),
            HirType::Map(key, value) => (
                "thaw_map_snapshot_entries",
                "__thaw_map_entries__",
                HirType::Tuple(vec![(**key).clone(), (**value).clone()]),
            ),
            HirType::Set(element) => ("thaw_map_snapshot_keys", "__thaw_set_values__", (**element).clone()),
            other => return Err(format!("{other:?} is not a native collection")),
        };
        let handle = if snapshot.is_empty() {
            collection
        } else {
            let buffer = self.builder
                .build_call(self.module.get_function(snapshot).unwrap(), &[collection.into()], "console_collection_snapshot")
                .map_err(|error| error.to_string())?
                .try_as_basic_value().basic()
                .ok_or("collection snapshot returned no value")?
                .into_pointer_value();
            self.compile_array_wrap(buffer)?
        };
        let entries = self.compile_native_array_to_json_with_undefined(handle, &element_type, preserve_undefined)?;
        let object = self.builder
            .build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "console_collection_json")
            .map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("thaw_json_object_new returned no value")?;
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_own").unwrap(),
            &[object.into()], "track_marshaled_collection",
        ).map_err(|error| error.to_string())?;
        let key = self.builder.build_global_string_ptr(wrapper_key, "console_collection_key")
            .map_err(|error| error.to_string())?;
        self.compile_json_object_set_native_with_undefined(
            object, key.as_pointer_value(), entries, &HirType::Json, preserve_undefined, true,
        )?;
        // A subclass instance handed on as its plain base type keeps its class name in console.log.
        if self.console_omit_absent_fields {
            self.compile_console_subclass_name(collection, object)?;
        }
        self.mark_fresh_owned_json_result(object)?;
        Ok(object)
    }

    /// Tags the console wrapper `object` with the subclass name remembered for `collection`.
    fn compile_console_subclass_name(
        &mut self,
        collection: PointerValue<'ctx>,
        object: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let name = self.builder.build_call(
            self.module.get_function("thaw_native_subclass_name").unwrap(),
            &[collection.into()], "native_subclass_name",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("subclass name query returned no value")?.into_pointer_value();
        let known = self.builder.build_is_not_null(name, "native_subclass_known")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let write = self.context.append_basic_block(function, "write_subclass_name");
        let done = self.context.append_basic_block(function, "subclass_name_done");
        self.builder.build_conditional_branch(known, write, done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(write);
        let key = self.builder.build_global_string_ptr("__thaw_class__", "console_class_key")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_object_set_string").unwrap(),
            &[object.into(), key.as_pointer_value().into(), name.into()], "set_console_class",
        ).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Field order for encoding; console output hides `#private` slots like Node does.
    fn visible_object_field_order(&self, fields: &[(String, HirType)]) -> Vec<usize> {
        let mut order = ecmascript_object_field_order(fields);
        // The native collection behind a `Map`/`Set` subclass is not an own property.
        order.retain(|&index| fields[index].0 != "__thaw_base");
        // An Error subclass assigns `name` in its constructor, after the declared fields.
        if self.encoding_stringify_errors
            && fields.first().is_some_and(|(marker, _)| {
                marker.strip_prefix("__thaw_class_identity_\u{1e}").is_some_and(|chain| {
                    chain.split('\u{1f}').any(thaw_hir::is_error_family_name)
                })
            })
        {
            if let Some(at) = order.iter().position(|&index| fields[index].0 == "name") {
                let name = order.remove(at);
                order.push(name);
            }
        }
        if self.console_omit_absent_fields {
            order.retain(|&index| !fields[index].0.starts_with("__thaw_private_"));
        }
        order
    }

    /// Encodes an object reached through a `Recursive` back-edge. Its layout is unbounded, so
    /// it cannot be unrolled inline: one internal helper per (layout, mode) encodes a node and
    /// calls itself for the nested back-edges, with its own clean function context.
    fn compile_recursive_to_json(
        &mut self,
        object: PointerValue<'ctx>,
        key: &str,
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let name = format!(
            "__thaw_recursive_json_{}_{}_{}",
            key.replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
            u8::from(preserve_undefined),
            u8::from(self.console_omit_absent_fields),
        );
        let pointer = self.context.ptr_type(AddressSpace::default());
        let helper = match self.module.get_function(&name) {
            Some(helper) => helper,
            None => {
                let layout = HirType::Recursive(key.to_string()).expand();
                if !matches!(layout, HirType::Object(_)) {
                    return Err(format!("unresolved recursive layout `{key}`"));
                }
                let helper = self.module.add_function(
                    &name,
                    pointer.fn_type(&[pointer.into()], false),
                    Some(Linkage::Internal),
                );
                let parent_block = self.builder.get_insert_block().ok_or("recursive encoder outside a function")?;
                let saved_variables = std::mem::take(&mut self.variables);
                let saved_scope_context = self.take_codegen_scope_context();
                let saved_for_iteration_frame_slots = std::mem::take(&mut self.for_iteration_frame_slots);
                let saved_catch_native_text = std::mem::take(&mut self.catch_native_text);
                let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
                let saved_arena_variables = std::mem::take(&mut self.arena_variables);
                let result = (|| -> Result<(), String> {
                    let entry = self.context.append_basic_block(helper, "entry");
                    self.builder.position_at_end(entry);
                    self.emit_stack_overflow_guard()?;
                    // A cyclic (or absurdly deep) structure must throw, not exhaust the stack.
                    let i64_type = self.context.i64_type();
                    let depth_cell = match self.module.get_global("__thaw_recursive_json_depth") {
                        Some(global) => global,
                        None => {
                            let global = self.module.add_global(i64_type, None, "__thaw_recursive_json_depth");
                            global.set_initializer(&i64_type.const_zero());
                            global.set_linkage(Linkage::Internal);
                            global
                        }
                    }.as_pointer_value();
                    let depth = self.builder.build_load(i64_type, depth_cell, "recursive_json_depth")
                        .map_err(|error| error.to_string())?.into_int_value();
                    let too_deep = self.builder.build_int_compare(
                        IntPredicate::UGT, depth, i64_type.const_int(4000, false), "recursive_json_too_deep")
                        .map_err(|error| error.to_string())?;
                    let overflow_bb = self.context.append_basic_block(helper, "recursive_json_overflow");
                    let body_bb = self.context.append_basic_block(helper, "recursive_json_body");
                    self.builder.build_conditional_branch(too_deep, overflow_bb, body_bb).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(overflow_bb);
                    self.builder.build_store(depth_cell, i64_type.const_zero()).map_err(|error| error.to_string())?;
                    self.compile_throw_builtin_error(
                        "TypeError",
                        "Converting circular structure to JSON (or structure too deep)",
                    )?;
                    if self.builder.get_insert_block().is_some_and(|block| block.get_terminator().is_none()) {
                        self.builder.build_unreachable().map_err(|error| error.to_string())?;
                    }
                    self.builder.position_at_end(body_bb);
                    let node = helper.get_first_param().ok_or("recursive encoder has no parameter")?.into_pointer_value();
                    let node_id = self.builder.build_ptr_to_int(node, i64_type, "recursive_node_id")
                        .map_err(|error| error.to_string())?;
                    let seen = self.builder.build_call(
                        self.module.get_function("thaw_recursive_lookup").unwrap(),
                        &[node_id.into()], "recursive_seen",
                    ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                        .ok_or("recursive lookup returned no value")?.into_pointer_value();
                    let is_cycle = self.builder.build_is_not_null(seen, "recursive_is_cycle")
                        .map_err(|error| error.to_string())?;
                    let cycle_bb = self.context.append_basic_block(helper, "recursive_json_cycle");
                    let fresh_bb = self.context.append_basic_block(helper, "recursive_json_fresh");
                    self.builder.build_conditional_branch(is_cycle, cycle_bb, fresh_bb).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(cycle_bb);
                    self.builder.build_return(Some(&seen)).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(fresh_bb);
                    let next = self.builder.build_int_add(depth, i64_type.const_int(1, false), "recursive_json_next")
                        .map_err(|error| error.to_string())?;
                    self.builder.build_store(depth_cell, next).map_err(|error| error.to_string())?;
                    self.recursive_register = Some(node);
                    self.bypass_recursive_route = true;
                    let json = self.compile_native_object_to_json_with_undefined(node, &layout, preserve_undefined)?;
                    self.recursive_register = None;
                    self.bypass_recursive_route = false;
                    self.builder.build_call(
                        self.module.get_function("thaw_recursive_unregister").unwrap(),
                        &[node_id.into()], "unregister_recursive_node",
                    ).map_err(|error| error.to_string())?;
                    self.builder.build_store(depth_cell, depth).map_err(|error| error.to_string())?;
                    self.builder.build_return(Some(&json)).map_err(|error| error.to_string())?;
                    Ok(())
                })();
                self.variables = saved_variables;
                self.replace_codegen_scope_context(saved_scope_context);
                self.for_iteration_frame_slots = saved_for_iteration_frame_slots;
                self.catch_native_text = saved_catch_native_text;
                self.variable_hir_types = saved_variable_hir_types;
                self.arena_variables = saved_arena_variables;
                self.builder.position_at_end(parent_block);
                result?;
                helper
            }
        };
        let json = self
            .builder
            .build_call(helper, &[object.into()], "recursive_json")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "recursive encoder returned no value".to_string())?;
        self.branch_on_pending_exception()?;
        Ok(json)
    }

    fn compile_native_object_to_json_with_undefined(
        &mut self,
        object: PointerValue<'ctx>,
        ty: &HirType,
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = ty else {
            return Err("dynamic object marshaling requires an object type".to_string());
        };
        // A layout that can reach itself is encoded through its recursive helper (also at the
        // top), so a reference cycle in the data is found instead of unrolled forever.
        let bypass = std::mem::take(&mut self.bypass_recursive_route);
        if !bypass && fields.iter().any(|(_, field)| field.contains_recursive()) {
            if let Some(key) = recursive_key_for_layout(ty) {
                return self.compile_recursive_to_json(object, &key, preserve_undefined);
            }
        }
        // console.log of a `Map`/`Set` subclass instance: the collection wrapper
        // the inspector knows, named after the most-derived class.
        if self.console_omit_absent_fields && fields.get(1).is_some_and(|(name, _)| name == "__thaw_base") {
            let base = self.compile_accessor_aware_field_read(object, ty, fields, 1)?;
            let wrapper = self.compile_native_collection_to_json(
                base.into_pointer_value(), &fields[1].1, preserve_undefined,
            )?;
            let class = fields[0].0.strip_prefix("__thaw_class_identity_\u{1e}")
                .and_then(|ancestry| ancestry.split('\u{1f}').next())
                .ok_or("native collection subclass has no identity marker")?;
            let key = self.builder.build_global_string_ptr("__thaw_class__", "console_class_key")
                .map_err(|error| error.to_string())?;
            let name = self.builder.build_global_string_ptr(class, "console_class_name")
                .map_err(|error| error.to_string())?;
            self.builder.build_call(
                self.module.get_function("thaw_json_object_set_string").unwrap(),
                &[wrapper.into(), key.as_pointer_value().into(), name.as_pointer_value().into()],
                "set_console_class",
            ).map_err(|error| error.to_string())?;
            // Own fields of the subclass follow the entries, as in Node.
            for index in self.visible_object_field_order(fields) {
                self.compile_native_object_json_field(
                    object, ty, fields, index, wrapper, preserve_undefined,
                )?;
            }
            return Ok(wrapper);
        }
        // Outside `console.log`, an `extends Array` instance is its items (as `JSON.stringify` sees it).
        if let Some((_, HirType::Array(element))) = fields.get(1).filter(|(name, _)| name == "__thaw_base") {
            let base = self.compile_accessor_aware_field_read(object, ty, fields, 1)?;
            return self.compile_native_array_to_json_with_undefined(
                base.into_pointer_value(), element, preserve_undefined,
            );
        }
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "dynamic_object_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_own").unwrap(),
            &[json.into()], "track_marshaled_object",
        ).map_err(|error| error.to_string())?;
        if let Some(node) = self.recursive_register.take() {
            let node = self.builder.build_ptr_to_int(node, self.context.i64_type(), "recursive_node")
                .map_err(|error| error.to_string())?;
            self.builder.build_call(
                self.module.get_function("thaw_recursive_register").unwrap(),
                &[node.into(), json.into()], "register_recursive_node",
            ).map_err(|error| error.to_string())?;
        }
        // Only ClassAlloc records this native Date identity. An ordinary
        // object with the same fields is still serialized as ordinary data.
        let date_identity = if fields.len() == 2
            && fields[0].0 == "__thaw_class_identity_\u{1e}Date"
            && fields[0].1 == HirType::Bool
            && fields[1].0 == "timestamp"
            && fields[1].1 == HirType::F64
        {
            let date_name = self.builder.build_global_string_ptr("Date", "native_date_identity_name")
                .map_err(|error| error.to_string())?;
            Some(self.builder.build_call(
                self.module.get_function("thaw_object_has_class_identity").unwrap(),
                &[object.into(), date_name.as_pointer_value().into()],
                "has_native_date_identity",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("native Date identity query returned no value")?.into_int_value())
        } else { None };
        // Snapshot all present own keys before a getter can mutate a later
        // marker. The physical field layout is static; the creation order is
        // per allocation and can differ after a fixed spread or rest copy.
        // `JSON.stringify` of an Error subclass lists declared fields in declaration order and `name`
        // (assigned in the constructor body) last, independent of assignment order.
        let stringify_error_record = self.encoding_stringify_errors
            && fields.first().is_some_and(|(marker, _)| {
                marker.strip_prefix("__thaw_class_identity_\u{1e}").is_some_and(|chain| {
                    chain.split('\u{1f}').any(thaw_hir::is_error_family_name)
                })
            });
        if stringify_error_record
            || !fields.iter().any(|(name, _)| name.starts_with("__thaw_class_identity_\u{1e}"))
        {
            // Only marker visibility/reveal can alter relative physical order.
            // A copied layout with no marker retains its static key order.
            for index in self.visible_object_field_order(fields) {
                self.compile_native_object_json_field(
                    object, ty, fields, index, json, preserve_undefined,
                )?;
            }
        } else {
        let i64_type = self.context.i64_type();
        let mut ranks = Vec::new();
        for (static_rank, index) in self.visible_object_field_order(fields).into_iter().enumerate() {
            let name = &fields[index].0;
            let key = self.compile_expr(&HirExpr::Lit(HirLit::Str(name.clone())))?
                .into_pointer_value();
            let rank = self.builder.build_call(
                self.module.get_function("thaw_object_order_rank").unwrap(),
                &[object.into(), key.into(),
                    i64_type.const_int(static_rank as u64, false).into()],
                "native_field_initial_rank",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("native field rank returned no value")?.into_int_value();
            let hidden = if name.as_bytes().contains(&0) {
                self.context.bool_type().const_zero()
            } else {
                let marker = self.builder.build_global_string_ptr(name, "native_field_initial_visibility")
                    .map_err(|error| error.to_string())?;
                self.builder.build_call(
                    self.module.get_function("thaw_object_marker_hidden").unwrap(),
                    &[object.into(), marker.as_pointer_value().into()],
                    "native_field_initially_hidden",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("native field visibility returned no value")?.into_int_value()
            };
            let initial = self.builder.build_select(
                hidden, i64_type.const_int(i64::MAX as u64, false), rank,
                "native_field_snapshot_rank",
            ).map_err(|error| error.to_string())?.into_int_value();
            let slot = self.builder.build_alloca(i64_type, "native_field_rank_slot")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(slot, initial).map_err(|error| error.to_string())?;
            ranks.push((index, slot));
        }
        for _ in 0..ranks.len() {
            let mut minimum = i64_type.const_int(i64::MAX as u64, false);
            for (_, slot) in &ranks {
                let rank = self.builder.build_load(i64_type, *slot, "native_field_rank")
                    .map_err(|error| error.to_string())?.into_int_value();
                let smaller = self.builder.build_int_compare(
                    IntPredicate::SLT, rank, minimum, "native_field_earlier",
                ).map_err(|error| error.to_string())?;
                minimum = self.builder.build_select(
                    smaller, rank, minimum, "native_field_minimum",
                ).map_err(|error| error.to_string())?.into_int_value();
            }
            let function = self.current_function();
            let done = self.context.append_basic_block(function, "native_order_step_done");
            let check = self.context.append_basic_block(function, "native_order_check");
            let absent = self.builder.build_int_compare(
                IntPredicate::EQ, minimum, i64_type.const_int(i64::MAX as u64, false),
                "native_order_no_more_keys",
            ).map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(absent, done, check)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(check);
            for (index, slot) in &ranks {
                let index = *index;
                let rank = self.builder.build_load(i64_type, *slot, "native_order_candidate")
                    .map_err(|error| error.to_string())?.into_int_value();
                let chosen = self.builder.build_int_compare(
                    IntPredicate::EQ, rank, minimum, "native_order_selected",
                ).map_err(|error| error.to_string())?;
                let write = self.context.append_basic_block(function, "native_order_write");
                let next = self.context.append_basic_block(function, "native_order_next");
                self.builder.build_conditional_branch(chosen, write, next)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(write);
            let skip_marker = if index == 0 {
                if let Some(identity) = date_identity {
                    let function = self.current_function();
                    let write = self.context.append_basic_block(function, "write_untrusted_date_marker");
                    let done = self.context.append_basic_block(function, "date_marker_done");
                    self.builder.build_conditional_branch(identity, done, write)
                        .map_err(|error| error.to_string())?;
                    self.builder.position_at_end(write);
                    Some(done)
                } else { None }
            } else { None };
                self.compile_native_object_json_field(
                    object, ty, fields, index, json, preserve_undefined,
                )?;
            if let Some(done) = skip_marker {
                self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
                self.builder.position_at_end(done);
            }
                self.builder.build_store(
                    *slot, i64_type.const_int(i64::MAX as u64, false),
                ).map_err(|error| error.to_string())?;
                self.builder.build_unconditional_branch(done)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(next);
            }
            self.builder.build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(done);
        }
        }
        // `q.stack = text` on a record with no `stack` slot lives in the runtime side table
        // (see `thaw_object_stack_assign`); an assigned one is an own enumerable key.
        let error_family_record = fields.first().is_some_and(|(marker, _)| {
            marker.strip_prefix("__thaw_class_identity_\u{1e}").is_some_and(|chain| {
                chain.split('\u{1f}').any(thaw_hir::is_error_family_name)
            })
        });
        if date_identity.is_none()
            && !error_family_record
            && !fields.iter().any(|(name, _)| matches!(name.as_str(), "stack" | "__thaw_getter_stack" | "__thaw_setter_stack"))
        {
            let function = self.current_function();
            let write = self.context.append_basic_block(function, "write_assigned_stack");
            let done = self.context.append_basic_block(function, "assigned_stack_done");
            let visible = self.builder.build_call(
                self.module.get_function("thaw_object_stack_visible").unwrap(),
                &[object.into()], "assigned_stack_visible",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("stack visibility query returned no value")?.into_int_value();
            self.builder.build_conditional_branch(visible, write, done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(write);
            let key = self.builder.build_global_string_ptr("stack", "assigned_stack_key")
                .map_err(|error| error.to_string())?;
            let empty = self.builder.build_global_string_ptr("", "assigned_stack_empty")
                .map_err(|error| error.to_string())?;
            let text = self.builder.build_call(
                self.module.get_function("thaw_object_stack_get").unwrap(),
                &[object.into(), empty.as_pointer_value().into()], "assigned_stack_text",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("stack text query returned no value")?;
            self.builder.build_call(
                self.module.get_function("thaw_json_object_set_string").unwrap(),
                &[json.into(), key.as_pointer_value().into(), text.into()],
                "set_assigned_stack",
            ).map_err(|error| error.to_string())?;
            self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
            self.builder.position_at_end(done);
        }
        if let Some(identity) = date_identity {
            // One native Date keeps one Json twin while that twin is alive,
            // so `a === a` and shared Date nodes survive the conversion.
            self.builder.build_call(
                self.module.get_function("thaw_json_adopt_native_date").unwrap(),
                &[object.into(), json.into(), identity.into()],
                "adopt_native_date_json",
            ).map_err(|error| error.to_string())?;
            self.builder.build_call(
                self.module.get_function("thaw_json_brand_wrapper_if").unwrap(),
                &[json.into(), identity.into()],
                "brand_native_date_json",
            ).map_err(|error| error.to_string())?;
        }
        // console.log names a class instance after its most-derived class; the
        // inspector reads the name back from this private key. Error-family
        // instances keep their own display and so are left unnamed.
        if self.console_omit_absent_fields {
            let marker = fields.iter()
                .find_map(|(name, _)| name.strip_prefix("__thaw_class_identity_\u{1e}"));
            // A native RegExp has no identity marker; its `{source, flags, lastIndex}` shape is
            // the type thaw-hir gives every RegExp (`regex_object_type`).
            let is_regexp = matches!(fields.as_slice(), [(a, HirType::Str), (b, HirType::Str), (c, HirType::F64)]
                if a == "source" && b == "flags" && c == "lastIndex");
            let class = if is_regexp { Some("RegExp") } else {
                marker.and_then(|ancestry| ancestry.split('\u{1f}').next())
                    .filter(|_| !marker.unwrap().split('\u{1f}').any(thaw_hir::is_error_family_name))
            };
            if let Some(class) = class {
                let key = self.builder.build_global_string_ptr("__thaw_class__", "console_class_key")
                    .map_err(|error| error.to_string())?;
                let name = self.builder.build_global_string_ptr(class, "console_class_name")
                    .map_err(|error| error.to_string())?;
                // The static type only says the record has the class's layout. A copy made by
                // `{ ...instance }` carries the hidden marker but was never allocated as an
                // instance, so it prints as a plain object.
                let named = if is_regexp {
                    None
                } else {
                    let identity = self.builder.build_call(
                        self.module.get_function("thaw_object_has_class_identity").unwrap(),
                        &[object.into(), name.as_pointer_value().into()],
                        "console_has_class_identity",
                    ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                        .ok_or("class identity query returned no value")?.into_int_value();
                    let function = self.current_function();
                    let write = self.context.append_basic_block(function, "write_console_class");
                    let done = self.context.append_basic_block(function, "console_class_done");
                    self.builder.build_conditional_branch(identity, write, done)
                        .map_err(|error| error.to_string())?;
                    self.builder.position_at_end(write);
                    Some(done)
                };
                self.builder.build_call(
                    self.module.get_function("thaw_json_object_set_string").unwrap(),
                    &[json.into(), key.as_pointer_value().into(), name.as_pointer_value().into()],
                    "set_console_class",
                ).map_err(|error| error.to_string())?;
                if let Some(done) = named {
                    self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
                    self.builder.position_at_end(done);
                }
            }
        }
        // Carries `object`'s own frozen/sealed/non-extensible state (if
        // any -- a no-op otherwise) across onto the freshly built `json`
        // pointer, so `Object.freeze({...})` assigned into an `any`-typed
        // binding stays observably frozen after crossing into its Json
        // representation, instead of the state silently vanishing because
        // `json` is a different allocation with a different identity.
        let state_key = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_state_key").unwrap(),
                &[json.into()],
                "json_encoded_state_key",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("JSON state key returned no value")?;
        self.builder
            .build_call(
                self.module.get_function("thaw_object_copy_state").unwrap(),
                &[object.into(), state_key.into()],
                "propagate_object_state",
            )
            .map_err(|error| error.to_string())?;
        self.mark_fresh_owned_json_result(json)?;
        Ok(json)
    }

    fn compile_native_object_getter(
        &mut self,
        object: PointerValue<'ctx>,
        fields: &[(String, HirType)],
        getter_index: usize,
    ) -> Result<(BasicValueEnum<'ctx>, HirType), String> {
        let getter_type = &fields[getter_index].1;
        let getter_type = match getter_type {
            HirType::OverloadedFunction(_, physical) => physical.as_ref(),
            other => other,
        };
        let (params, ret) = match getter_type {
            HirType::Function(params, ret) | HirType::CallableFunction(params, _, _, ret) => {
                (params, ret.as_ref())
            }
            other => return Err(format!("native object getter has type {other:?}")),
        };
        let pointer = self.compile_field_ptr_from_pointer(object, fields, getter_index)?;
        let closure = self
            .builder
            .build_load(self.context.ptr_type(AddressSpace::default()), pointer, "getter_closure")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        // The registered-accessor call wraps the receiver the way the getter's `this` parameter
        // expects (a `NonArrowReceiver` payload, not the raw object pointer).
        let value = self
            .compile_registered_accessor_call(closure, params, ret, &[object.into()], "getter_call")?
            .ok_or("native object getter did not return a value")?;
        Ok((value, ret.clone()))
    }

    fn live_native_projection_ledger(&self) -> Option<NativeHandleLedger<'ctx>> {
        let (function_name, ledger) = self.live_native_projection.as_ref()?;
        (self.current_function().get_name().to_string_lossy().as_ref() == function_name.as_str()).then_some(*ledger)
    }

    fn append_live_native_symbol_handle(
        &mut self,
        handle: IntValue<'ctx>,
        ledger: NativeHandleLedger<'ctx>,
    ) -> Result<(), String> {
        let i64_type = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let old = self.builder.build_load(ptr_ty, ledger.buffer, "native_symbol_ledger_old")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let capacity = self.builder.build_load(i64_type, ledger.capacity, "native_symbol_ledger_capacity")
            .map_err(|error| error.to_string())?.into_int_value();
        let old_present = self.builder.build_is_not_null(old, "native_symbol_ledger_has_buffer")
            .map_err(|error| error.to_string())?;
        let read_length = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_read_length");
        let empty = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_empty");
        let decide = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_decide");
        self.builder.build_conditional_branch(old_present, read_length, empty)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(read_length);
        let current_length = self.builder.build_load(i64_type, old, "native_symbol_ledger_length")
            .map_err(|error| error.to_string())?.into_int_value();
        self.builder.build_unconditional_branch(decide).map_err(|error| error.to_string())?;
        let length_end = self.builder.get_insert_block().ok_or("ledger length block missing")?;
        self.builder.position_at_end(empty);
        self.builder.build_unconditional_branch(decide).map_err(|error| error.to_string())?;
        let empty_end = self.builder.get_insert_block().ok_or("ledger empty block missing")?;
        self.builder.position_at_end(decide);
        let length = self.builder.build_phi(i64_type, "native_symbol_ledger_current_length")
            .map_err(|error| error.to_string())?;
        let zero = i64_type.const_zero();
        length.add_incoming(&[(&current_length, length_end), (&zero, empty_end)]);
        let length = length.as_basic_value().into_int_value();
        let has_room = self.builder.build_int_compare(
            IntPredicate::ULT, length, capacity, "native_symbol_ledger_has_capacity",
        ).map_err(|error| error.to_string())?;
        let append_in_place = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_append_in_place");
        let grow = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_grow");
        let done = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_appended");
        self.builder.build_conditional_branch(has_room, append_in_place, grow)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(append_in_place);
        let next_length = self.builder.build_int_add(length, i64_type.const_int(1, false), "native_symbol_ledger_next_length")
            .map_err(|error| error.to_string())?;
        let value_offset = self.builder.build_int_mul(next_length, i64_type.const_int(8, false), "native_symbol_ledger_value_offset")
            .map_err(|error| error.to_string())?;
        let value_slot = unsafe {
            self.builder.build_in_bounds_gep(self.context.i8_type(), old, &[value_offset], "native_symbol_ledger_value_slot")
        }.map_err(|error| error.to_string())?;
        let value_slot = self.builder.build_pointer_cast(value_slot, ptr_ty, "native_symbol_ledger_value_slot_typed")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(value_slot, handle).map_err(|error| error.to_string())?;
        let length_slot = self.builder.build_pointer_cast(old, ptr_ty, "native_symbol_ledger_length_slot")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(length_slot, next_length).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;

        self.builder.position_at_end(grow);
        let empty_capacity = self.builder.build_int_compare(
            IntPredicate::EQ, capacity, i64_type.const_zero(), "native_symbol_ledger_first_capacity",
        ).map_err(|error| error.to_string())?;
        let doubled = self.builder.build_int_mul(capacity, i64_type.const_int(2, false), "native_symbol_ledger_doubled_capacity")
            .map_err(|error| error.to_string())?;
        let next_capacity = self.builder.build_select(
            empty_capacity, i64_type.const_int(4, false), doubled, "native_symbol_ledger_next_capacity",
        ).map_err(|error| error.to_string())?.into_int_value();
        let grew = self.builder.build_int_compare(
            IntPredicate::UGT, next_capacity, capacity, "native_symbol_ledger_capacity_no_overflow",
        ).map_err(|error| error.to_string())?;
        let maximum = i64_type.const_int((u64::MAX - 8) / 8, false);
        let fits_bytes = self.builder.build_int_compare(
            IntPredicate::ULE, next_capacity, maximum, "native_symbol_ledger_size_fits",
        ).map_err(|error| error.to_string())?;
        let valid_capacity = self.builder.build_and(grew, fits_bytes, "native_symbol_ledger_capacity_valid")
            .map_err(|error| error.to_string())?;
        let allocate = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_allocate");
        let invalid_capacity = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_capacity_overflow");
        self.builder.build_conditional_branch(valid_capacity, allocate, invalid_capacity)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid_capacity);
        self.builder.build_call(
            self.module.get_function("thaw_js_release_handle").unwrap(), &[handle.into()],
            "release_unappended_native_symbol_overflow",
        ).map_err(|error| error.to_string())?;
        self.release_native_handle_ledger(ledger)?;
        self.compile_throw_type_error("Native Symbol handle ledger capacity overflow")?;

        self.builder.position_at_end(allocate);
        let capacity_bytes = self.builder.build_int_mul(next_capacity, i64_type.const_int(8, false), "native_symbol_ledger_capacity_bytes")
            .map_err(|error| error.to_string())?;
        let allocation_bytes = self.builder.build_int_add(capacity_bytes, i64_type.const_int(8, false), "native_symbol_ledger_allocation_bytes")
            .map_err(|error| error.to_string())?;
        let allocation = self.builder.build_call(
            self.module.get_function("thaw_arena_alloc").unwrap(),
            &[allocation_bytes.into(), i64_type.const_int(8, false).into()],
            "native_symbol_ledger_allocation",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native symbol ledger allocation returned no value")?.into_pointer_value();
        let allocated = self.builder.build_is_not_null(allocation, "native_symbol_ledger_allocated")
            .map_err(|error| error.to_string())?;
        let allocated_ok = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_allocated_ok");
        let allocation_failed = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_allocation_failed");
        self.builder.build_conditional_branch(allocated, allocated_ok, allocation_failed)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(allocation_failed);
        self.builder.build_call(
            self.module.get_function("thaw_js_release_handle").unwrap(), &[handle.into()],
            "release_unappended_native_symbol_allocation_failure",
        ).map_err(|error| error.to_string())?;
        self.release_native_handle_ledger(ledger)?;
        self.compile_throw_type_error("Unable to allocate native Symbol handle ledger")?;

        self.builder.position_at_end(allocated_ok);
        let next_length = self.builder.build_int_add(length, i64_type.const_int(1, false), "native_symbol_ledger_grown_length")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(allocation, next_length).map_err(|error| error.to_string())?;
        let copy = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_copy_old_handles");
        let copied = self.context.append_basic_block(self.current_function(), "native_symbol_ledger_copy_complete");
        let has_old_handles = self.builder.build_int_compare(
            IntPredicate::NE, length, i64_type.const_zero(), "native_symbol_ledger_has_old_handles",
        ).map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_old_handles, copy, copied)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(copy);
        let source_data = unsafe {
            self.builder.build_in_bounds_gep(self.context.i8_type(), old, &[i64_type.const_int(8, false)], "native_symbol_ledger_old_data")
        }.map_err(|error| error.to_string())?;
        let destination_data = unsafe {
            self.builder.build_in_bounds_gep(self.context.i8_type(), allocation, &[i64_type.const_int(8, false)], "native_symbol_ledger_new_data")
        }.map_err(|error| error.to_string())?;
        let copy_bytes = self.builder.build_int_mul(length, i64_type.const_int(8, false), "native_symbol_ledger_copy_bytes")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("memcpy").unwrap(),
            &[destination_data.into(), source_data.into(), copy_bytes.into()],
            "native_symbol_ledger_copy",
        ).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(copied).map_err(|error| error.to_string())?;
        self.builder.position_at_end(copied);
        let new_value_offset = self.builder.build_int_mul(next_length, i64_type.const_int(8, false), "native_symbol_ledger_new_value_offset")
            .map_err(|error| error.to_string())?;
        let new_value_slot = unsafe {
            self.builder.build_in_bounds_gep(self.context.i8_type(), allocation, &[new_value_offset], "native_symbol_ledger_new_value_slot")
        }.map_err(|error| error.to_string())?;
        let new_value_slot = self.builder.build_pointer_cast(new_value_slot, ptr_ty, "native_symbol_ledger_new_value_slot_typed")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(new_value_slot, handle).map_err(|error| error.to_string())?;
        self.builder.build_store(ledger.buffer, allocation).map_err(|error| error.to_string())?;
        self.builder.build_store(ledger.capacity, next_capacity).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn compile_live_symbol_json_placeholder(
        &mut self,
        value: PointerValue<'ctx>,
        ledger: NativeHandleLedger<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let nonnull = self.builder.build_is_not_null(value, "native_symbol_projection_string_present")
            .map_err(|error| error.to_string())?;
        let valid = self.context.append_basic_block(self.current_function(), "native_symbol_projection_string_valid");
        let invalid = self.context.append_basic_block(self.current_function(), "native_symbol_projection_string_invalid");
        self.builder.build_conditional_branch(nonnull, valid, invalid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.release_native_handle_ledger(ledger)?;
        self.compile_throw_type_error("Native Symbol had no string payload")?;
        self.builder.position_at_end(valid);
        let wire = self.builder.build_call(
            self.module.get_function("thaw_json_array_new").unwrap(), &[],
            "native_symbol_projection_wire",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("symbol projection wire allocation returned no value")?;
        self.builder.build_call(
            self.module.get_function("thaw_json_array_push_native_wtf8_string").unwrap(),
            &[wire.into(), value.into()], "native_symbol_projection_wire_push",
        ).map_err(|error| error.to_string())?;
        let encoded = self.builder.build_call(
            self.module.get_function("thaw_json_stringify").unwrap(), &[wire.into()],
            "native_symbol_projection_wire_json",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("symbol projection stringifier returned no value")?;
        let encoded = self.compile_check_json_stringify_error_with_cleanup(encoded, &[wire])?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_event_key_handle_result").unwrap(),
            &[encoded.into(), self.context.i8_type().const_int(1, false).into()],
            "native_symbol_projection_handle_result",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("symbol projection bridge returned no result")?.into_struct_value();
        let handle = self.builder.build_extract_value(result, 0, "native_symbol_projection_handle")
            .map_err(|error| error.to_string())?.into_int_value();
        let error = self.builder.build_extract_value(result, 1, "native_symbol_projection_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(), &[encoded.into()],
            "destroy_native_symbol_projection_wire",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(), &[wire.into()],
            "destroy_native_symbol_projection_array",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;

        self.append_live_native_symbol_handle(handle, ledger)?;
        self.compile_dynamic_value_placeholder_unchecked(handle.into())
    }

    fn compile_json_object_set_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        field_type: &HirType,
    ) -> Result<(), String> {
        self.compile_json_object_set_native_with_undefined(json, key, value, field_type, false, false)
    }

    /// A `Str` handed to a QuickJS dynamic call: a framed `Error` string must reach JS as a real
    /// Error, so it is converted through `thaw_json_receiver_string` (owned Json) instead of a plain
    /// string field/element. Native-only marshaling (console, N-API) keeps the plain string.
    fn compile_dynamic_argument_string(
        &mut self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_receiver_string").unwrap(),
                &[value.into()],
                "dynamic_string_argument",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("string argument conversion returned no value")?;
        self.compile_check_json_host_error(json, Some("thaw_json_destroy"))
    }

    fn compile_json_object_set_native_with_undefined(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        mut value: BasicValueEnum<'ctx>,
        field_type: &HirType,
        preserve_undefined: bool,
        owned: bool,
    ) -> Result<(), String> {
        let field_type = match field_type {
            HirType::OverloadedFunction(_, physical) => physical.as_ref(),
            other => other,
        };
        match field_type {
            HirType::Optional(payload) => {
                return self.compile_json_object_set_tagged(
                    json,
                    key,
                    value.into_struct_value(),
                    payload,
                    JsonTaggedKind::Optional,
                    preserve_undefined,
                );
            }
            HirType::Nullable(payload) => {
                return self.compile_json_object_set_tagged(
                    json,
                    key,
                    value.into_struct_value(),
                    payload,
                    JsonTaggedKind::Nullable,
                    preserve_undefined,
                );
            }
            HirType::Nullish(payload) => {
                return self.compile_json_object_set_tagged(
                    json,
                    key,
                    value.into_struct_value(),
                    payload,
                    JsonTaggedKind::Nullish,
                    preserve_undefined,
                );
            }
            HirType::Union(elements) => {
                return self.compile_json_object_set_union(
                    json,
                    key,
                    value.into_struct_value(),
                    elements,
                    preserve_undefined,
                );
            }
            HirType::Undefined => {
                if !preserve_undefined {
                    return Ok(());
                }
                value = self.compile_napi_undefined_json()?;
            }
            HirType::Null => value = self.compile_json_null()?,
            _ => {}
        }
        let dynamic_string = matches!(field_type, HirType::Str) && self.compiling_quickjs_dynamic_arguments;
        let owned = owned
            || dynamic_string
            || matches!(field_type, HirType::Null | HirType::Undefined | HirType::I64
                | HirType::Map(..) | HirType::Set(_)
                | HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) | HirType::Recursive(_)
                | HirType::JsValue | HirType::Function(..) | HirType::CallableFunction(..))
            || (matches!(field_type, HirType::Json | HirType::Dictionary(_)) && value.is_int_value())
            || (matches!(field_type, HirType::Symbol) && self.live_native_projection_ledger().is_some());
        let mut setter = match field_type {
                HirType::F64 => "thaw_json_object_set_number",
                HirType::Str if dynamic_string => {
                    value = self.compile_dynamic_argument_string(value)?;
                    "thaw_json_object_set_json"
                }
                HirType::Str => "thaw_json_object_set_string",
                HirType::Symbol if self.live_native_projection_ledger().is_some() => {
                    let ledger = self.live_native_projection_ledger().unwrap();
                    value = self.compile_live_symbol_json_placeholder(value.into_pointer_value(), ledger)?;
                    "thaw_json_object_set_json"
                }
                HirType::Bool => {
                    value = self
                        .builder
                        .build_int_z_extend(
                            value.into_int_value(),
                            self.context.i8_type(),
                            "marshal_object_bool",
                        )
                        .map_err(|error| error.to_string())?
                        .into();
                    "thaw_json_object_set_bool"
                }
                HirType::Json | HirType::Dictionary(_) => {
                    // A field whose *declared* type is the generic `Json`
                    // (an unresolvable/fallback npm parameter type, e.g.
                    // zod's `optional`) can still carry a real `JsValue`
                    // handle at runtime -- thaw-hir's `coerce_to_declared`
                    // passes one through unchanged rather than erroring
                    // (see its own doc comment). `Json`/`Dictionary` are
                    // always pointers (see `basic_type`); a `JsValue` is
                    // always an `i64`, so this is an unambiguous way to
                    // tell the two apart here.
                    if value.is_int_value() {
                        value = self.compile_dynamic_value_placeholder(value)?;
                    }
                    "thaw_json_object_set_json"
                }
                HirType::Bytes => {
                    value = self.compile_native_bytes_to_json(value.into_pointer_value())?;
                    "thaw_json_object_set_json"
                }
                HirType::Array(element) => {
                    value = self.compile_native_array_to_json_with_undefined(
                        value.into_pointer_value(),
                        element,
                        preserve_undefined,
                    )?;
                    "thaw_json_object_set_json"
                }
                HirType::Tuple(elements) => {
                    value = self.compile_native_tuple_to_json_with_undefined(
                        value.into_pointer_value(),
                        elements,
                        preserve_undefined,
                    )?;
                    "thaw_json_object_set_json"
                }
                HirType::Object(_) => {
                    value = self.compile_native_object_to_json_with_undefined(
                        value.into_pointer_value(),
                        field_type,
                        preserve_undefined,
                    )?;
                    "thaw_json_object_set_json"
                }
                HirType::Null | HirType::Undefined => "thaw_json_object_set_json",
                HirType::I64 => {
                    value = self.compile_json_bigint(value)?;
                    "thaw_json_object_set_json"
                }
                HirType::Map(..) | HirType::Set(_) if self.console_omit_absent_fields => {
                    value = self.compile_native_collection_to_json(value.into_pointer_value(), field_type, preserve_undefined)?;
                    "thaw_json_object_set_json"
                }
                // `JSON.stringify` serialises a Map/Set field as `{}` (no own
                // enumerable properties).
                HirType::Map(..) | HirType::Set(_) => {
                    value = self
                        .builder
                        .build_call(self.module.get_function("thaw_json_object_new").unwrap(), &[], "json_empty_collection")
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .ok_or("thaw_json_object_new returned no value")?;
                    "thaw_json_object_set_json"
                }
                HirType::JsValue => {
                    value = self.compile_dynamic_value_placeholder(value)?;
                    "thaw_json_object_set_json"
                }
                HirType::Function(..) | HirType::CallableFunction(..) if self.console_omit_absent_fields => {
                    value = self.compile_console_function_sentinel(value.into_pointer_value())?;
                    "thaw_json_object_set_json"
                }
                HirType::Function(params, ret) => {
                    value = self.compile_register_native_callback_from_closure(
                        value.into_pointer_value(),
                        params,
                        ret,
                    )?;
                    value = self.compile_dynamic_value_placeholder(value)?;
                    "thaw_json_object_set_json"
                }
                // Same idea as `HirType::Function` just above, but for a
                // callback field with optional/rest parameters (real
                // trigger: `better-sqlite3`'s own `DatabaseOptions.verbose?:
                // (message?: unknown, ...additionalArgs: unknown[]) => void`
                // option) -- flattens to the same fixed-`params` shape
                // `compile_register_native_callback_from_closure` expects,
                // appending a rest parameter as a trailing `Array(element)`
                // entry, mirroring `dynamic_host/typed_calls.rs`'s own
                // identical `CallableFunction` -> ABI-params flattening for
                // a top-level dynamic-call callback argument.
                HirType::CallableFunction(params, _, rest, ret) => {
                    let mut abi_params = params.clone();
                    let has_rest = rest.is_some();
                    if let Some(rest) = rest {
                        abi_params.push(HirType::Array(rest.clone()));
                    }
                    value = self.compile_register_native_callback_from_closure_with_rest(
                        value.into_pointer_value(),
                        &abi_params,
                        ret,
                        has_rest,
                        json_callback_wants_graph(&abi_params),
                    )?;
                    value = self.compile_dynamic_value_placeholder(value)?;
                    "thaw_json_object_set_json"
                }
                HirType::Recursive(key) => {
                    value = self.compile_recursive_to_json(
                        value.into_pointer_value(), key, preserve_undefined,
                    )?;
                    "thaw_json_object_set_json"
                }
                other => return Err(format!("unsupported dynamic object field {other:?}")),
            };
        if owned && setter == "thaw_json_object_set_json" {
            setter = "thaw_json_object_set_json_owned";
        }
        let consumes_json = setter == "thaw_json_object_set_json_owned";
        self.builder
            .build_call(
                self.module.get_function(setter).unwrap(),
                &[json.into(), key.into(), value.into()],
                "set_dynamic_object_field",
            )
            .map_err(|error| error.to_string())?;
        self.compile_check_json_host_error(json, None)?;
        if consumes_json {
            self.revoke_fresh_owned_json_result(value)?;
        }
        Ok(())
    }

    /// Encodes a `JsValue` (an opaque, permanent id into QuickJS's own
    /// retained-value table -- there is no JSON representation of "a live
    /// JS object") as a `{"__thaw_js_handle_id__": <id>}` placeholder
    /// object, which the QuickJS-side JSON reviver (see its doc comment in
    /// `dates.js`) splices back into the real value the moment the args
    /// JSON this placeholder sits in gets parsed on the other end -- bare,
    /// or nested inside an object/array literal at any depth, the same way
    /// that reviver already reconstructs a `Date`. Unlike a per-call
    /// index into a side channel, the handle's own id is permanent and
    /// meaningful on its own, so no extra plumbing back to the call site
    /// is needed here. Only meaningful while compiling a QuickJS-backed
    /// dynamic call's own arguments (`compile_typed_dynamic_call` sets
    /// `compiling_quickjs_dynamic_arguments` before marshaling them);
    /// anywhere else -- console.log formatting, a `Dictionary` literal, an
    /// N-API call (which has no such reviver) -- there is no receiving end
    /// for this placeholder, so this fails clearly instead of silently
    /// discarding the live value. Real-world example: zod's
    /// `z.object({ name: z.string() })`, whose `z.string()` argument is
    /// itself a `JsValue` (a live `ZodString` schema instance) nested
    /// inside the object-literal argument passed to `z.object`.
    fn compile_dynamic_value_placeholder(
        &mut self,
        handle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if !self.compiling_quickjs_dynamic_arguments {
            // No QuickJS reviver downstream: embed the live value itself (a Json Host lease).
            self.uses_quickjs = true;
            self.uses_quickjs_handles = true;
            self.tracks_owned_json_roots = true;
            let json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_from_borrowed_handle").unwrap(),
                    &[handle.into_int_value().into()],
                    "dynamic_value_json_host",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("live JSON host conversion returned no value")?;
            return self.compile_check_json_host_error(json, Some("thaw_json_destroy"));
        }
        self.compile_dynamic_value_placeholder_unchecked(handle)
    }

    /// The gate-free core of `compile_dynamic_value_placeholder`, for a
    /// call site that already knows (by construction, not by the
    /// per-compilation `compiling_quickjs_dynamic_arguments` flag) that a
    /// reviver exists downstream -- see `build_call_with`'s own use of
    /// this, which reaches it only for an *ordinary* function call
    /// (never N-API, which rejects a `JsValue` argument outright well
    /// before codegen).
    pub(super) fn compile_dynamic_value_placeholder_unchecked(
        &mut self,
        handle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let handle = self
            .builder
            .build_unsigned_int_to_float(
                handle.into_int_value(),
                self.context.f64_type(),
                "dynamic_handle_id",
            )
            .map_err(|error| error.to_string())?;
        let placeholder = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "dynamic_handle_placeholder",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let key = self
            .builder
            .build_global_string_ptr("__thaw_js_handle_id__", "dynamic_handle_placeholder_key")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_object_set_number")
                    .unwrap(),
                &[placeholder.into(), key.as_pointer_value().into(), handle.into()],
                "dynamic_handle_placeholder_id",
            )
            .map_err(|error| error.to_string())?;
        let branded = self.builder.build_call(
            self.module.get_function("thaw_json_brand_wrapper").unwrap(),
            &[placeholder.into()], "brand_dynamic_handle_placeholder",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_json_brand_wrapper returned no value")?;
        Ok(branded)
    }

    /// Sets a `Union`-typed object field's JSON value -- the same
    /// per-member tag dispatch as `compile_json_array_push_union` (see
    /// its own doc comment), just recursing into
    /// `compile_json_object_set_native_with_undefined` for the same
    /// `key` instead of pushing onto an array. Real-world example:
    /// camelcase's own `Options` fields being read out and marshaled
    /// for a dynamic call, several of which are themselves optional
    /// unions once resolved.
    fn compile_json_object_set_union(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        value: StructValue<'ctx>,
        elements: &[HirType],
        preserve_undefined: bool,
    ) -> Result<(), String> {
        if elements.is_empty() {
            return Err("cannot serialize an empty union to JSON".into());
        }
        let tag = self
            .builder
            .build_extract_value(value, 0, "json_object_union_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "json_object_union_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "json_object_union_set");
        for (index, member) in elements.iter().enumerate() {
            let matched = self
                .context
                .append_basic_block(function, "json_object_union_member");
            if index + 1 == elements.len() {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            } else {
                let next = self
                    .context
                    .append_basic_block(function, "json_object_union_next");
                let is_match = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "json_object_union_tag_match",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_match, matched, next)
                    .map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(matched);
            let member_value = self.unpack_union_payload(payload, member)?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                member_value,
                member,
                preserve_undefined,
                false,
            )?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if index + 1 != elements.len() {
                let next = matched
                    .get_next_basic_block()
                    .ok_or("union JSON dispatch lost its next comparison block")?;
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        Ok(())
    }

    fn compile_json_object_set_tagged(
        &mut self,
        json: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
        tagged: StructValue<'ctx>,
        payload_type: &HirType,
        kind: JsonTaggedKind,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        let three_state = matches!(kind, JsonTaggedKind::Nullish);
        let absent_is_null = matches!(kind, JsonTaggedKind::Nullable);
        let tag = self
            .builder
            .build_extract_value(tagged, 0, "json_object_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(tagged, 1, "json_object_payload")
            .map_err(|error| error.to_string())?;
        let present = if three_state {
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_zero(),
                    "json_object_has_value",
                )
                .map_err(|error| error.to_string())?
        } else {
            tag
        };
        let function = self.current_function();
        let value_block = self.context.append_basic_block(function, "json_object_value");
        let absent_block = self.context.append_basic_block(function, "json_object_absent");
        let done = self.context.append_basic_block(function, "json_object_tagged_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        if three_state {
            let null_block = self.context.append_basic_block(function, "json_object_null");
            let undefined_block = self
                .context
                .append_basic_block(function, "json_object_undefined");
            let is_null = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_int(1, false),
                    "json_object_is_null",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(is_null, null_block, undefined_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(null_block);
            let null = self.compile_json_null()?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                null,
                &HirType::Json,
                preserve_undefined,
                true,
            )?;
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(undefined_block);
            // `T | null | undefined` holds an explicit `undefined` value, which
            // Node prints (`{ nullish: undefined }`); only a bare optional
            // property is omitted when absent.
            if preserve_undefined {
                let undefined = self.compile_napi_undefined_json()?;
                self.compile_json_object_set_native_with_undefined(
                    json,
                    key,
                    undefined,
                    &HirType::Json,
                    true,
                    true,
                )?;
            }
        } else if absent_is_null {
            let null = self.compile_json_null()?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                null,
                &HirType::Json,
                preserve_undefined,
                true,
            )?;
        } else if preserve_undefined
            && (!self.console_omit_absent_fields || self.encoding_class_instance_fields)
        {
            let undefined = self.compile_napi_undefined_json()?;
            self.compile_json_object_set_native_with_undefined(
                json,
                key,
                undefined,
                &HirType::Json,
                true,
                true,
            )?;
        }
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        self.compile_json_object_set_native_with_undefined(
            json,
            key,
            payload,
            payload_type,
            preserve_undefined,
            false,
        )?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn compile_native_array_to_json(
        &mut self,
        array: PointerValue<'ctx>,
        element_type: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_native_array_to_json_with_undefined(array, element_type, false)
    }

    /// A native byte buffer as its `Buffer.toJSON()` shape, `{ "type": "Buffer", "data": [..] }`.
    /// A function nested in a `console.log` value: the sentinel the inspector prints as `[Function: name]`.
    pub(super) fn compile_console_function_sentinel(
        &mut self,
        closure: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder.build_call(
            self.module.get_function("thaw_json_function_sentinel").unwrap(),
            &[closure.into()], "console_function_sentinel",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or_else(|| "function sentinel returned no value".to_string())
    }

    /// A byte buffer handle as a fresh `f64`-slot array handle (a copy), for
    /// the element-wise JSON encoders.
    fn compile_bytes_widened_handle(
        &mut self,
        buffer: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let record = self.compile_array_data(buffer)?;
        let data = self
            .builder
            .build_call(self.module.get_function("thaw_bytes_to_array").unwrap(), &[record.into()], "bytes_widen")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_bytes_to_array returned no value")?
            .into_pointer_value();
        self.compile_array_wrap(data)
    }

    fn compile_native_bytes_to_json(
        &mut self,
        value: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let object = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "bytes_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let ty_key = self
            .builder
            .build_global_string_ptr("type", "bytes_type_key")
            .map_err(|error| error.to_string())?;
        let ty_value = self
            .builder
            .build_global_string_ptr("Buffer", "bytes_type_value")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_object_set_string").unwrap(),
                &[object.into(), ty_key.as_pointer_value().into(), ty_value.as_pointer_value().into()],
                "set_bytes_type",
            )
            .map_err(|error| error.to_string())?;
        let widened = self.compile_bytes_widened_handle(value)?;
        let data = self.compile_native_array_to_json_with_undefined(widened, &HirType::F64, true)?;
        let data_key = self
            .builder
            .build_global_string_ptr("data", "bytes_data_key")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_object_set_json").unwrap(),
                &[object.into(), data_key.as_pointer_value().into(), data.into()],
                "set_bytes_data",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[data.into()],
                "destroy_bytes_data",
            )
            .map_err(|error| error.to_string())?;
        Ok(object)
    }

    fn compile_native_array_to_json_with_undefined(
        &mut self,
        array: PointerValue<'ctx>,
        element_type: &HirType,
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let element_type = match element_type {
            HirType::OverloadedFunction(_, physical) => physical.as_ref(),
            other => other,
        };
        // `array` is a handle (see `compile_array_wrap`'s doc comment);
        // unwrap it once here so every existing byte-level access below
        // keeps working against the raw buffer unchanged.
        let array_handle = array;
        let array = self.compile_array_data(array_handle)?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "console_array_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_array_new returned no value")?;
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_own").unwrap(),
            &[json.into()], "track_marshaled_array",
        ).map_err(|error| error.to_string())?;
        let i64_type = self.context.i64_type();
        let length = self
            .builder
            .build_load(i64_type, array, "console_array_length")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let entry = self
            .builder
            .get_insert_block()
            .ok_or("array conversion has no current block")?;
        let condition = self.context.append_basic_block(function, "console_array_next");
        let body = self.context.append_basic_block(function, "console_array_element");
        let done = self.context.append_basic_block(function, "console_array_done");
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(condition);
        let index = self
            .builder
            .build_phi(i64_type, "console_array_index")
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&i64_type.const_zero(), entry)]);
        let has_element = self
            .builder
            .build_int_compare(
                IntPredicate::ULT,
                index.as_basic_value().into_int_value(),
                length,
                "console_array_has_element",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_element, body, done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(body);
        let state = self.compile_array_index_state(
            array_handle,
            index.as_basic_value().into_int_value(),
        )?;
        let present = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state,
                self.context.i8_type().const_int(1, false),
                "console_array_has_value",
            )
            .map_err(|error| error.to_string())?;
        let present_block = self.context.append_basic_block(function, "console_array_present");
        let hole_block = self.context.append_basic_block(function, "console_array_hole");
        let pushed = self.context.append_basic_block(function, "console_array_pushed");
        self.builder
            .build_conditional_branch(present, present_block, hole_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(present_block);
        let offset = self
            .builder
            .build_int_mul(
                index.as_basic_value().into_int_value(),
                i64_type.const_int(array_element_storage_bytes(element_type)?, false),
                "console_array_element_offset",
            )
            .map_err(|error| error.to_string())?;
        let offset = self
            .builder
            .build_int_add(
                offset,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "console_array_payload_offset",
            )
            .map_err(|error| error.to_string())?;
        let pointer = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    array,
                    &[offset],
                    "console_array_element_pointer",
                )
                .map_err(|error| error.to_string())?
        };
        let value = self
            .builder
            .build_load(
                self.basic_type(element_type)?,
                pointer,
                "console_array_element_value",
            )
            .map_err(|error| error.to_string())?;
        self.compile_json_array_push_native_with_undefined(
            json,
            value,
            element_type,
            preserve_undefined,
        )?;
        self.builder
            .build_unconditional_branch(pushed)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(hole_block);
        if preserve_undefined {
            let absent = self.context.append_basic_block(function, "console_array_absent");
            let explicit_undefined = self.context.append_basic_block(function, "console_array_explicit_undefined");
            let is_absent = self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    state,
                    self.context.i8_type().const_zero(),
                    "console_array_is_hole",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(is_absent, absent, explicit_undefined)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(absent);
            self.builder
                .build_call(
                    self.module.get_function("thaw_json_array_push_hole").unwrap(),
                    &[json.into()],
                    "array_hole_json_hole",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(pushed)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(explicit_undefined);
            let undefined = self.compile_napi_undefined_json()?;
            self.compile_json_array_push_owned(json, undefined)?;
            self.builder
                .build_unconditional_branch(pushed)
                .map_err(|error| error.to_string())?;
        } else {
            let null = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_null").unwrap(),
                    &[],
                    "array_hole_json_null",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_null returned no value")?;
            self.compile_json_array_push_owned(json, null)?;
            self.builder
                .build_unconditional_branch(pushed)
                .map_err(|error| error.to_string())?;
        }

        self.builder.position_at_end(pushed);
        let next = self
            .builder
            .build_int_add(
                index.as_basic_value().into_int_value(),
                i64_type.const_int(1, false),
                "console_array_increment",
            )
            .map_err(|error| error.to_string())?;
        let body_end = self
            .builder
            .get_insert_block()
            .ok_or("array conversion lost its body block")?;
        self.builder
            .build_unconditional_branch(condition)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next, body_end)]);

        self.builder.position_at_end(done);
        self.mark_fresh_owned_json_result(json)?;
        Ok(json)
    }

    #[allow(dead_code)]
    fn compile_native_tuple_to_json(
        &mut self,
        tuple: PointerValue<'ctx>,
        element_types: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_native_tuple_to_json_with_undefined(tuple, element_types, false)
    }

    fn compile_native_tuple_to_json_with_undefined(
        &mut self,
        tuple: PointerValue<'ctx>,
        element_types: &[HirType],
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // See `compile_native_array_to_json_with_undefined` -- `tuple` is a
        // handle, unwrap once up front.
        let tuple = self.compile_array_data(tuple)?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "console_tuple_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_array_new returned no value")?;
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_own").unwrap(),
            &[json.into()], "track_marshaled_tuple",
        ).map_err(|error| error.to_string())?;
        let stride = tuple_element_storage_bytes(element_types)?;
        for (index, element_type) in element_types.iter().enumerate() {
            let offset = self
                .context
                .i64_type()
                .const_int(checked_array_offset(stride, index)?, false);
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        tuple,
                        &[offset],
                        "console_tuple_element_pointer",
                    )
                    .map_err(|error| error.to_string())?
            };
            let value = self
                .builder
                .build_load(
                    self.basic_type(element_type)?,
                    pointer,
                    "console_tuple_element_value",
                )
                .map_err(|error| error.to_string())?;
            self.compile_json_array_push_native_with_undefined(
                json,
                value,
                element_type,
                preserve_undefined,
            )?;
        }
        self.mark_fresh_owned_json_result(json)?;
        Ok(json)
    }

    fn compile_json_array_push_native(
        &mut self,
        json: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
        element_type: &HirType,
    ) -> Result<(), String> {
        self.compile_json_array_push_native_with_undefined(json, value, element_type, false)
    }

    /// A Json BigInt for a native `i64` (the runtime owns the allocation).
    fn compile_json_bigint(&mut self, value: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_call(self.module.get_function("thaw_json_receiver_bigint").unwrap(), &[value.into()], "json_bigint")
            .map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or_else(|| "thaw_json_receiver_bigint returned no value".to_string())
    }

    fn compile_json_array_push_native_with_undefined(
        &mut self,
        json: BasicValueEnum<'ctx>,
        mut value: BasicValueEnum<'ctx>,
        element_type: &HirType,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        match element_type {
            HirType::Optional(payload) => {
                return self.compile_json_array_push_tagged(
                    json,
                    value.into_struct_value(),
                    payload,
                    false,
                    false,
                    preserve_undefined,
                );
            }
            HirType::Nullable(payload) => {
                return self.compile_json_array_push_tagged(
                    json,
                    value.into_struct_value(),
                    payload,
                    true,
                    false,
                    preserve_undefined,
                );
            }
            HirType::Nullish(payload) => {
                return self.compile_json_array_push_tagged(
                    json,
                    value.into_struct_value(),
                    payload,
                    false,
                    true,
                    preserve_undefined,
                );
            }
            HirType::Union(elements) => {
                return self.compile_json_array_push_union(
                    json,
                    value.into_struct_value(),
                    elements,
                    preserve_undefined,
                );
            }
            HirType::Undefined if preserve_undefined => {
                let undefined = self.compile_napi_undefined_json()?;
                return self.compile_json_array_push_owned(json, undefined);
            }
            HirType::Null | HirType::Undefined | HirType::Void => {
                value = self.compile_json_null()?;
            }
            _ => {}
        }
        let dynamic_string = matches!(element_type, HirType::Str) && self.compiling_quickjs_dynamic_arguments;
        let generated_owned = dynamic_string || matches!(element_type,
            HirType::Null | HirType::Undefined | HirType::Void | HirType::I64
            | HirType::Map(..) | HirType::Set(_)
            | HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) | HirType::Recursive(_)
            | HirType::JsValue | HirType::Function(..) | HirType::CallableFunction(..))
            || (matches!(element_type, HirType::Json | HirType::Dictionary(_)) && value.is_int_value())
            || (matches!(element_type, HirType::Symbol) && self.live_native_projection_ledger().is_some());
        let push = match element_type {
            HirType::F64 => "thaw_json_array_push_number",
            HirType::Str if dynamic_string => {
                value = self.compile_dynamic_argument_string(value)?;
                "thaw_json_array_push_json"
            }
            HirType::Str => "thaw_json_array_push_string",
            HirType::Symbol if self.live_native_projection_ledger().is_some() => {
                let ledger = self.live_native_projection_ledger().unwrap();
                value = self.compile_live_symbol_json_placeholder(value.into_pointer_value(), ledger)?;
                "thaw_json_array_push_json"
            }
            HirType::Bool => {
                value = self
                    .builder
                    .build_int_z_extend(
                        value.into_int_value(),
                        self.context.i8_type(),
                        "console_array_bool",
                    )
                    .map_err(|error| error.to_string())?
                    .into();
                "thaw_json_array_push_bool"
            }
            HirType::Json | HirType::Dictionary(_) => {
                // See the identical check in
                // `compile_json_object_set_native_with_undefined`: a
                // `Json`-declared slot can carry a real `JsValue` handle at
                // runtime (an unambiguous `i64` vs. pointer distinction).
                // `_unchecked`: this array-push helper is reached from
                // `compile_typed_dynamic_argument` (a dynamic call's own
                // argument list, where the reviver exists) as well as from
                // `compile_json_array_push_owned`, both of which are
                // already inside a QuickJS-backed dynamic-call argument
                // marshaling context -- the same reason that sibling uses
                // the gate-free variant.
                if value.is_int_value() {
                    value = self.compile_dynamic_value_placeholder_unchecked(value)?;
                }
                "thaw_json_array_push_json"
            }
            HirType::Bytes => {
                value = self.compile_native_bytes_to_json(value.into_pointer_value())?;
                "thaw_json_array_push_json"
            }
            HirType::Array(nested) => {
                value = self.compile_native_array_to_json_with_undefined(
                    value.into_pointer_value(),
                    nested,
                    preserve_undefined,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Tuple(elements) => {
                value = self.compile_native_tuple_to_json_with_undefined(
                    value.into_pointer_value(),
                    elements,
                    preserve_undefined,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Object(_) => {
                value = self.compile_native_object_to_json_with_undefined(
                    value.into_pointer_value(), element_type, preserve_undefined,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Recursive(key) => {
                value = self.compile_recursive_to_json(
                    value.into_pointer_value(), key, preserve_undefined,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Null | HirType::Undefined | HirType::Void => {
                "thaw_json_array_push_json"
            }
            // A bigint element becomes a Json BigInt (printed `123n`; `JSON.stringify` rejects it).
            HirType::I64 => {
                value = self.compile_json_bigint(value)?;
                "thaw_json_array_push_json"
            }
            HirType::Map(..) | HirType::Set(_) if self.console_omit_absent_fields => {
                value = self.compile_native_collection_to_json(value.into_pointer_value(), element_type, preserve_undefined)?;
                "thaw_json_array_push_json"
            }
            HirType::JsValue => {
                value = self.compile_dynamic_value_placeholder(value)?;
                "thaw_json_array_push_json"
            }
            HirType::Function(..) | HirType::CallableFunction(..) if self.console_omit_absent_fields => {
                value = self.compile_console_function_sentinel(value.into_pointer_value())?;
                "thaw_json_array_push_json"
            }
            HirType::Function(params, ret) | HirType::CallableFunction(params, _, _, ret) => {
                // A rest callable's physical ABI is its fixed prefix plus one packed `Array`.
                let (abi_params, has_rest) = match element_type {
                    HirType::CallableFunction(params, _, Some(rest), _) => {
                        let mut abi = params.clone();
                        abi.push(HirType::Array(rest.clone()));
                        (abi, true)
                    }
                    _ => (params.clone(), false),
                };
                value = self.compile_register_native_callback_from_closure_with_rest(
                    value.into_pointer_value(),
                    &abi_params,
                    ret,
                    has_rest,
                    json_callback_wants_graph(&abi_params),
                )?;
                value = self.compile_dynamic_value_placeholder(value)?;
                "thaw_json_array_push_json"
            }
            other => {
                return Err(format!(
                    "cannot serialize collection element {other:?} to JSON"
                ))
            }
        };
        self.builder
            .build_call(
                self.module.get_function(push).unwrap(),
                &[json.into(), value.into()],
                "console_array_push",
            )
            .map_err(|error| error.to_string())?;
        if generated_owned {
            self.builder.build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[value.into()], "destroy_marshaled_array_element",
            ).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Pushes a `Union` value onto a JSON array (or, by the same
    /// mechanism, packs one into a dynamic call's own JSON argument
    /// array -- both share this function through
    /// `compile_json_array_push_native_with_undefined`): reads the
    /// value's own runtime tag and, exactly like `compile_console_union`
    /// does to print one, dispatches to whichever member is actually
    /// active, unpacks its payload (`unpack_union_payload`, the same
    /// helper `HirExpr::UnionValue`'s own codegen uses), and recurses
    /// into this same push logic with that member's real type -- so a
    /// union nested inside another union, or one whose active member is
    /// itself an array/object/etc., serializes correctly too. Real-world
    /// example: camelcase's own `camelCase(input: string | readonly
    /// string[], options?): string`.
    fn compile_json_array_push_union(
        &mut self,
        json: BasicValueEnum<'ctx>,
        value: StructValue<'ctx>,
        elements: &[HirType],
        preserve_undefined: bool,
    ) -> Result<(), String> {
        if elements.is_empty() {
            return Err("cannot serialize an empty union to JSON".into());
        }
        let tag = self
            .builder
            .build_extract_value(value, 0, "json_union_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "json_union_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "json_union_pushed");
        for (index, member) in elements.iter().enumerate() {
            let matched = self.context.append_basic_block(function, "json_union_member");
            if index + 1 == elements.len() {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            } else {
                let next = self.context.append_basic_block(function, "json_union_next");
                let is_match = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "json_union_tag_match",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_match, matched, next)
                    .map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(matched);
            let member_value = self.unpack_union_payload(payload, member)?;
            self.compile_json_array_push_native_with_undefined(
                json,
                member_value,
                member,
                preserve_undefined,
            )?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if index + 1 != elements.len() {
                let next = matched
                    .get_next_basic_block()
                    .ok_or("union JSON dispatch lost its next comparison block")?;
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        Ok(())
    }

    fn compile_json_null(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_json_null").unwrap(),
                &[],
                "json_null",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_null returned no value".to_string())
    }

    fn compile_json_array_push_tagged(
        &mut self,
        json: BasicValueEnum<'ctx>,
        tagged: StructValue<'ctx>,
        payload_type: &HirType,
        absent_is_null: bool,
        three_state: bool,
        preserve_undefined: bool,
    ) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(tagged, 0, "json_collection_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(tagged, 1, "json_collection_payload")
            .map_err(|error| error.to_string())?;
        let present = if three_state {
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_zero(),
                    "json_collection_has_value",
                )
                .map_err(|error| error.to_string())?
        } else {
            tag
        };
        let function = self.current_function();
        let value_block = self.context.append_basic_block(function, "json_collection_value");
        let absent_block = self
            .context
            .append_basic_block(function, "json_collection_absent");
        let done = self.context.append_basic_block(function, "json_collection_tagged_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let absent = if preserve_undefined && !absent_is_null {
            if three_state {
                let null_block = self.context.append_basic_block(function, "json_collection_null");
                let undefined_block = self
                    .context
                    .append_basic_block(function, "json_collection_undefined");
                let absent_done = self
                    .context
                    .append_basic_block(function, "json_collection_absent_done");
                let is_null = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(1, false),
                        "json_collection_is_null",
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
                let null_end = self.builder.get_insert_block().ok_or("lost JSON null block")?;
                self.builder.position_at_end(undefined_block);
                let undefined = self.compile_napi_undefined_json()?;
                self.builder
                    .build_unconditional_branch(absent_done)
                    .map_err(|error| error.to_string())?;
                let undefined_end = self
                    .builder
                    .get_insert_block()
                    .ok_or("lost JSON undefined block")?;
                self.builder.position_at_end(absent_done);
                let result = self
                    .builder
                    .build_phi(
                        self.context.ptr_type(AddressSpace::default()),
                        "json_collection_absent_value",
                    )
                    .map_err(|error| error.to_string())?;
                result.add_incoming(&[(&null, null_end), (&undefined, undefined_end)]);
                result.as_basic_value()
            } else {
                self.compile_napi_undefined_json()?
            }
        } else {
            self.compile_json_null()?
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_array_push_json")
                    .unwrap(),
                &[json.into(), absent.into()],
                "push_collection_absent",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[absent.into()], "destroy_marshaled_array_absence",
        ).map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        self.compile_json_array_push_native_with_undefined(
            json,
            payload,
            payload_type,
            preserve_undefined,
        )?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

}

/// The recursive-layout key whose registered layout is exactly `ty`, if any.
fn recursive_key_for_layout(ty: &HirType) -> Option<String> {
    thaw_hir::recursive_key_for_layout(ty)
}
