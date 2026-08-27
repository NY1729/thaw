impl<'ctx> HirCompiler<'ctx> {
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
        let get_fn = self.module.get_function("thaw_json_get").unwrap();
        let call = self
            .builder
            .build_call(
                get_fn,
                &[obj_val.into(), key_global.as_pointer_value().into()],
                "json_get",
            )
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_get did not return a value".to_string())
    }

    fn compile_json_key(
        &mut self,
        obj: &HirExpr,
        key: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj = self.compile_expr(obj)?;
        let key = self.compile_expr(key)?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_get").unwrap(),
                &[obj.into(), key.into()],
                "json_key",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_get did not return a value".to_string())
    }

    fn compile_json_set(
        &mut self,
        object: &HirExpr,
        key: &HirExpr,
        value: &HirExpr,
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let object = self.compile_expr(object)?;
        let key = self.compile_expr(key)?;
        let result = self.compile_expr(value)?;
        let mut argument = result;
        let setter = match element {
            HirType::F64 => "thaw_json_object_set_number",
            HirType::Str => "thaw_json_object_set_string",
            HirType::Bool => {
                argument = self
                    .builder
                    .build_int_z_extend(
                        result.into_int_value(),
                        self.context.i8_type(),
                        "dictionary_bool",
                    )
                    .map_err(|error| error.to_string())?
                    .into();
                "thaw_json_object_set_bool"
            }
            HirType::Json | HirType::Dictionary(_) => "thaw_json_object_set_json",
            other => return Err(format!("unsupported dictionary value type {other:?}")),
        };
        self.builder
            .build_call(
                self.module.get_function(setter).unwrap(),
                &[object.into(), key.into(), argument.into()],
                "dictionary_set",
            )
            .map_err(|error| error.to_string())?;
        Ok(result)
    }

    fn compile_json_delete(
        &mut self,
        object: &HirExpr,
        key: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let object = self.compile_expr(object)?;
        let key = self.compile_expr(key)?;
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
        self.builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                deleted,
                self.context.i8_type().const_zero(),
                "json_delete",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
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
            let mut value = self.compile_expr(expression)?;
            let setter = match element {
                HirType::F64 => "thaw_json_object_set_number",
                HirType::Str => "thaw_json_object_set_string",
                HirType::Bool => {
                    value = self
                        .builder
                        .build_int_z_extend(
                            value.into_int_value(),
                            self.context.i8_type(),
                            "dictionary_bool",
                        )
                        .map_err(|error| error.to_string())?
                        .into();
                    "thaw_json_object_set_bool"
                }
                HirType::Json | HirType::Dictionary(_) => "thaw_json_object_set_json",
                other => return Err(format!("unsupported dictionary value type {other:?}")),
            };
            let key = self
                .builder
                .build_global_string_ptr(name, "dictionary_key")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function(setter).unwrap(),
                    &[object.into(), key.as_pointer_value().into(), value.into()],
                    "dictionary_set",
                )
                .map_err(|error| error.to_string())?;
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
        let idx_i64 = self
            .builder
            .build_float_to_signed_int(idx_val, self.context.i64_type(), "jsonidx")
            .map_err(|e| e.to_string())?;
        let index_fn = self.module.get_function("thaw_json_index").unwrap();
        let call = self
            .builder
            .build_call(index_fn, &[obj_val.into(), idx_i64.into()], "json_index")
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_index did not return a value".to_string())
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
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{fn_name}` did not return a value"))
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
        let zero = self.context.i8_type().const_int(0, false);
        self.builder
            .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "json_as_bool")
            .map(Into::into)
            .map_err(|e| e.to_string())
    }

}
