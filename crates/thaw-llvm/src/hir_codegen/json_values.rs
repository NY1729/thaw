impl<'ctx> HirCompiler<'ctx> {
    fn compile_decode_quickjs_graph(
        &mut self,
        text: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_decode_graph(text, true)
    }

    fn compile_decode_graph(
        &mut self,
        text: BasicValueEnum<'ctx>,
        owned_text: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let decoded = self.builder.build_call(
            self.module.get_function("thaw_json_graph_decode").unwrap(),
            &[text.into()], "decode_quickjs_graph",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("QuickJS graph decode returned no value")?;
        if owned_text {
            self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[text.into()], "destroy_quickjs_graph_text")
                .map_err(|error| error.to_string())?;
        }
        let status = self.builder.build_call(
            self.module.get_function("thaw_json_take_graph_error").unwrap(),
            &[], "quickjs_graph_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("QuickJS graph status returned no value")?.into_int_value();
        let failed = self.builder.build_int_compare(
            inkwell::IntPredicate::NE, status, self.context.i8_type().const_zero(),
            "quickjs_graph_failed",
        ).map_err(|error| error.to_string())?;
        let invalid = self.context.append_basic_block(self.current_function(), "quickjs_graph_invalid");
        let valid = self.context.append_basic_block(self.current_function(), "quickjs_graph_valid");
        self.builder.build_conditional_branch(failed, invalid, valid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
            &[decoded.into()], "destroy_invalid_quickjs_graph")
            .map_err(|error| error.to_string())?;
        self.compile_throw_type_error("Invalid JavaScript result graph")?;
        self.builder.position_at_end(valid);
        Ok(decoded)
    }
    /// A live QuickJS-backed Json operation can execute a getter or coercion.
    /// Consume its error immediately, while the owned result is still in
    /// scope, and use the same pending-exception channel as dynamic calls.
    fn compile_check_json_host_error(
        &mut self,
        result: BasicValueEnum<'ctx>,
        destroy_result: Option<&str>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let error = self.builder.build_call(
            self.module.get_function("thaw_json_take_host_error").unwrap(),
            &[], "json_host_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("host error query returned no value")?.into_pointer_value();
        let function = self.current_function();
        let failed = self.context.append_basic_block(function, "json_host_failed");
        let succeeded = self.context.append_basic_block(function, "json_host_succeeded");
        let has_error = self.builder.build_is_not_null(error, "json_host_has_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_error, failed, succeeded)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        if let Some(symbol) = destroy_result {
            self.builder.build_call(self.module.get_function(symbol).unwrap(),
                &[result.into()], "destroy_failed_json_host_result")
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        self.builder.position_at_end(succeeded);
        Ok(result)
    }

    /// `json.field`, via thaw-std's `thaw_json_get`.
    fn compile_json_get(
        &mut self,
        obj: &HirExpr,
        field: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let key_global = self
            .builder
            .build_global_string_ptr(field, "jsonkey")
            .map_err(|e| e.to_string())?;
        let get_fn = self
            .module
            .get_function(if matches!(obj, HirExpr::Var(name) if name.starts_with("__thaw_json_wrap_obj_")) {
                "thaw_json_take"
            } else {
                "thaw_json_get"
            })
            .unwrap();
        let call = self
            .builder
            .build_call(
                get_fn,
                &[obj_val.into(), key_global.as_pointer_value().into()],
                "json_get",
            )
            .map_err(|e| e.to_string())?;
        let value = call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_get did not return a value".to_string())?;
        self.compile_check_json_host_error(value, Some("thaw_json_destroy"))
    }

    fn compile_json_key(
        &mut self,
        obj: &HirExpr,
        key: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj = self.compile_expr(obj)?;
        let key = self.compile_expr(key)?;
        let value = self.builder
            .build_call(
                self.module.get_function("thaw_json_get").unwrap(),
                &[obj.into(), key.into()],
                "json_key",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_get did not return a value".to_string())?;
        self.compile_check_json_host_error(value, Some("thaw_json_destroy"))
    }

    fn compile_json_set(
        &mut self,
        object: &HirExpr,
        key: &HirExpr,
        value: &HirExpr,
        element: &HirType,
        preserve_undefined: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let owned = matches!(object, HirExpr::Var(name) if name.starts_with("__thaw_json_wrap_obj_"));
        let object = self.compile_expr(object)?;
        let key = self.compile_expr(key)?;
        let result = self.compile_expr(value)?;
        if !owned {
            // PutValue happens after RHS evaluation. A nullish reference
            // fails here, before frozen/sealed state or the native write.
            self.compile_guard_json_non_nullish(object, "Cannot set properties of null or undefined")?;
            // `owned` means this write is populating a brand-new object
            // literal still under construction (never observable/frozen
            // yet) -- only a write through an existing binding needs the
            // freeze/seal guard.
            self.compile_guard_json_write(object, key.into_pointer_value())?;
        }
        self.compile_json_object_set_native_with_undefined(
            object,
            key.into_pointer_value(),
            result,
            element,
            preserve_undefined,
            owned,
        )?;
        Ok(result)
    }

    /// `Object.freeze` blocks every write to an existing binding;
    /// `Object.seal`/`preventExtensions` (without `freeze`) block only
    /// *adding* a genuinely new key -- updating an existing key stays
    /// allowed. Mirrors `HirExpr::PropAssign`'s equivalent guard for a
    /// statically `Object`-typed receiver, but also needs a `thaw_json_
    /// has_own` presence check first, since a `Json` write can add a key
    /// a compiled `Object`'s fixed layout never could. See
    /// `thaw_object_state`'s query encoding (0 extensible, 2 frozen) in
    /// `thaw-runtime/src/runtime/native_values/objects.rs`.
    fn compile_guard_json_write(
        &mut self,
        object: BasicValueEnum<'ctx>,
        key: PointerValue<'ctx>,
    ) -> Result<(), String> {
        if !object.is_pointer_value() {
            return Ok(());
        }
        let object_state = self.module.get_function("thaw_object_state").unwrap();
        let state_key = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_state_key").unwrap(),
                &[object.into()],
                "json_write_state_key",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("JSON state key returned no value")?;
        let function = self.current_function();
        let check_extensible = self
            .context
            .append_basic_block(function, "json_write_check_extensible");
        let check_has_own = self
            .context
            .append_basic_block(function, "json_write_check_has_own");
        let frozen_blocked = self.context.append_basic_block(function, "json_write_frozen");
        let extension_blocked = self
            .context
            .append_basic_block(function, "json_write_not_extensible");
        let allowed = self.context.append_basic_block(function, "json_write_allowed");

        let frozen = self
            .builder
            .build_call(
                object_state,
                &[state_key.into(), self.context.i8_type().const_int(2, false).into()],
                "json_write_is_frozen",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_object_state returned no value")?
            .into_int_value();
        self.builder
            .build_conditional_branch(frozen, frozen_blocked, check_extensible)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(check_extensible);
        let extensible = self
            .builder
            .build_call(
                object_state,
                &[state_key.into(), self.context.i8_type().const_zero().into()],
                "json_write_is_extensible",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_object_state returned no value")?
            .into_int_value();
        self.builder
            .build_conditional_branch(extensible, allowed, check_has_own)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(check_has_own);
        let has_own = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_has_own").unwrap(),
                &[object.into(), key.into()],
                "json_write_has_own",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_has_own returned no value")?
            .into_int_value();
        let has_own = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                has_own,
                self.context.i8_type().const_zero(),
                "json_write_has_own_bool",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_own, allowed, extension_blocked)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(frozen_blocked);
        self.compile_throw_type_error("Cannot assign to read only property of object")?;

        self.builder.position_at_end(extension_blocked);
        self.compile_throw_type_error("Cannot add property, object is not extensible")?;

        self.builder.position_at_end(allowed);
        Ok(())
    }

    fn compile_json_delete(
        &mut self,
        object: &HirExpr,
        key: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let object = self.compile_expr(object)?;
        let key = self.compile_expr(key)?;
        self.compile_guard_json_non_nullish(object, "Cannot convert undefined or null to object")?;
        let deleted = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_delete").unwrap(),
                &[object.into(), key.into()],
                "json_delete_u8",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_delete did not return a value")?
            .into_int_value();
        self.compile_check_json_host_error(deleted.into(), None)?;
        let succeeded = self.builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                deleted,
                self.context.i8_type().const_zero(),
                "json_delete",
            )
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let failed = self.context.append_basic_block(function, "json_delete_failed");
        let allowed = self.context.append_basic_block(function, "json_delete_allowed");
        self.builder.build_conditional_branch(succeeded, allowed, failed)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        self.compile_throw_type_error("Cannot delete property")?;
        self.builder.position_at_end(allowed);
        Ok(succeeded.into())
    }

    fn compile_json_object_lit(
        &mut self,
        fields: &[(String, HirExpr)],
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let object = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "dictionary",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new did not return a value")?;
        for (name, expression) in fields {
            let value = self.compile_expr(expression)?;
            let key = self
                .builder
                .build_global_string_ptr(name, "dictionary_key")
                .map_err(|error| error.to_string())?;
            self.compile_json_object_set_native(
                object,
                key.as_pointer_value(),
                value,
                element,
            )?;
        }
        Ok(object)
    }

    /// `json[index]`, via thaw-std's `thaw_json_index`.
    fn compile_json_index(
        &mut self,
        obj: &HirExpr,
        index: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let idx_val = self.compile_expr(index)?.into_float_value();
        let key = self
            .builder
            .build_call(
                self.module.get_function("thaw_number_to_string").unwrap(),
                &[idx_val.into()],
                "json_index_key",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_number_to_string returned no value")?;
        let index_fn = self.module.get_function("thaw_json_index").unwrap();
        let call = self
            .builder
            .build_call(
                index_fn,
                &[obj_val.into(), idx_val.into(), key.into()],
                "json_index",
            )
            .map_err(|e| e.to_string())?;
        let value = call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_index did not return a value".to_string())?;
        self.compile_check_json_host_error(value, Some("thaw_json_destroy"))
    }

    fn compile_json_index_set(
        &mut self,
        object: &HirExpr,
        index: &HirExpr,
        value: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let object = self.compile_expr(object)?;
        let index = self.compile_expr(index)?.into_float_value();
        let key = self
            .builder
            .build_call(
                self.module.get_function("thaw_number_to_string").unwrap(),
                &[index.into()],
                "json_set_key",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_number_to_string returned no value")?;
        let value = self.compile_expr(value)?;
        self.compile_guard_json_non_nullish(object, "Cannot set properties of null or undefined")?;
        self.compile_guard_json_write(object, key.into_pointer_value())?;
        let result = self.builder
            .build_call(
                self.module.get_function("thaw_json_index_set").unwrap(),
                &[object.into(), index.into(), key.into(), value.into()],
                "json_index_set",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_index_set did not return a value".to_string())?;
        self.compile_check_json_host_error(result, None)
    }

    /// `Number(json)`/`String(json)`/`Boolean(json)`.
    fn compile_json_as(
        &mut self,
        inner: &HirExpr,
        fn_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let function = self.module.get_function(fn_name).unwrap();
        let call = self
            .builder
            .build_call(function, &[val.into()], "json_as")
            .map_err(|e| e.to_string())?;
        let value = call.try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{fn_name}` did not return a value"))?;
        self.compile_check_json_host_error(value, (fn_name == "thaw_json_as_string")
            .then_some("thaw_cstring_destroy"))
    }

    /// `Boolean(json)`. Separate from `compile_json_as`: `thaw_json_as_bool`
    /// returns `i8` (0/1), not `i1`, so the result needs converting to
    /// match how `HirType::Bool` is represented everywhere else.
    fn compile_json_as_bool(&mut self, inner: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let function = self.module.get_function("thaw_json_as_bool").unwrap();
        let call = self
            .builder
            .build_call(function, &[val.into()], "json_as_bool_u8")
            .map_err(|e| e.to_string())?;
        let u8_val = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_as_bool did not return a value")?
            .into_int_value();
        self.compile_check_json_host_error(u8_val.into(), None)?;
        let zero = self.context.i8_type().const_int(0, false);
        self.builder
            .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "json_as_bool")
            .map(Into::into)
            .map_err(|e| e.to_string())
    }

}
