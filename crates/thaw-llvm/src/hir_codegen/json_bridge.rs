impl<'ctx> HirCompiler<'ctx> {
    fn compile_json_backend_call(
        &mut self,
        args: &[HirExpr],
        backend_symbol: &str,
        source_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [name, call_args] = args else {
            return Err(format!(
                "{source_name} expects exactly two arguments (name, args)"
            ));
        };
        let name_val = self.compile_expr(name)?;
        let args_json_val = self.compile_expr(call_args)?;

        self.compile_json_backend_values(name_val, args_json_val, backend_symbol)
    }

    fn compile_json_backend_values(
        &mut self,
        name_val: BasicValueEnum<'ctx>,
        args_json_val: BasicValueEnum<'ctx>,
        backend_symbol: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let stringify_fn = self.module.get_function("thaw_json_stringify").unwrap();
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

        let call_fn = self.module.get_function(backend_symbol).unwrap();
        let result = self
            .builder
            .build_call(
                call_fn,
                &[name_val.into(), args_json_str.into()],
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
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|e| e.to_string())?;
        self.branch_on_pending_exception()?;

        let parse_fn = self.module.get_function("thaw_json_parse").unwrap();
        self.builder
            .build_call(parse_fn, &[result_json_str.into()], "call_dynamic_result")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse did not return a value".to_string())
    }

    fn compile_json_as_value(
        &mut self,
        json: BasicValueEnum<'ctx>,
        symbol: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_call(
                self.module.get_function(symbol).unwrap(),
                &[json.into()],
                symbol,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{symbol} returned no value"))
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

    fn compile_native_object_to_json(
        &mut self,
        object: PointerValue<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = ty else {
            return Err("dynamic object marshaling requires an object type".to_string());
        };
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
        for (index, (name, field_ty)) in fields.iter().enumerate() {
            let offset = self
                .context
                .i64_type()
                .const_int(object_field_offset(fields, index), false);
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), object, &[offset], "marshal_field")
                    .map_err(|error| error.to_string())?
            };
            let mut value = self
                .builder
                .build_load(self.basic_type(field_ty)?, pointer, "marshal_field_value")
                .map_err(|error| error.to_string())?;
            let setter = match field_ty {
                HirType::F64 => "thaw_json_object_set_number",
                HirType::Str => "thaw_json_object_set_string",
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
                HirType::Json | HirType::Dictionary(_) => "thaw_json_object_set_json",
                HirType::Array(element) => {
                    value = self.compile_native_array_to_json(
                        value.into_pointer_value(),
                        element,
                    )?;
                    "thaw_json_object_set_json"
                }
                HirType::Object(_) => {
                    value =
                        self.compile_native_object_to_json(value.into_pointer_value(), field_ty)?;
                    "thaw_json_object_set_json"
                }
                other => return Err(format!("unsupported dynamic object field {other:?}")),
            };
            let key = self
                .builder
                .build_global_string_ptr(name, "dynamic_object_key")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function(setter).unwrap(),
                    &[json.into(), key.as_pointer_value().into(), value.into()],
                    "set_dynamic_object_field",
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(json)
    }

    fn compile_native_array_to_json(
        &mut self,
        array: PointerValue<'ctx>,
        element_type: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
        let offset = self
            .builder
            .build_int_mul(
                index.as_basic_value().into_int_value(),
                i64_type.const_int(array_element_storage_bytes(element_type), false),
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
        let mut value = self
            .builder
            .build_load(
                self.basic_type(element_type)?,
                pointer,
                "console_array_element_value",
            )
            .map_err(|error| error.to_string())?;
        let push = match element_type {
            HirType::F64 => "thaw_json_array_push_number",
            HirType::Str => "thaw_json_array_push_string",
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
            HirType::Json | HirType::Dictionary(_) => "thaw_json_array_push_json",
            HirType::Array(nested) => {
                value = self.compile_native_array_to_json(value.into_pointer_value(), nested)?;
                "thaw_json_array_push_json"
            }
            HirType::Object(_) => {
                value = self.compile_native_object_to_json(value.into_pointer_value(), element_type)?;
                "thaw_json_array_push_json"
            }
            other => return Err(format!("console.log cannot serialize array element {other:?}")),
        };
        self.builder
            .build_call(
                self.module.get_function(push).unwrap(),
                &[json.into(), value.into()],
                "console_array_push",
            )
            .map_err(|error| error.to_string())?;
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
        Ok(json)
    }

    fn compile_json_to_native_object(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirType::Object(fields) = ty else {
            return Err("dynamic result requires an object type".to_string());
        };
        let object = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(object_storage_bytes(fields), false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "dynamic_result_object",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        for (index, (name, field_ty)) in fields.iter().enumerate() {
            let key = self
                .builder
                .build_global_string_ptr(name, "dynamic_result_key")
                .map_err(|error| error.to_string())?;
            let field_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_get").unwrap(),
                    &[json.into(), key.as_pointer_value().into()],
                    "dynamic_result_field_json",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let field = match field_ty {
                HirType::F64 => self.compile_json_as_value(field_json, "thaw_json_as_number")?,
                HirType::Str => self.compile_json_as_value(field_json, "thaw_json_as_string")?,
                HirType::Bool => self.compile_json_as_bool_value(field_json)?,
                HirType::Json => field_json,
                HirType::Array(element) if **element == HirType::F64 => self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_json_to_number_array")
                            .unwrap(),
                        &[field_json.into()],
                        "dynamic_result_array_field",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap(),
                HirType::Object(_) => self.compile_json_to_native_object(field_json, field_ty)?,
                other => return Err(format!("unsupported dynamic result field {other:?}")),
            };
            let offset = self
                .context
                .i64_type()
                .const_int(object_field_offset(fields, index), false);
            let pointer = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), object, &[offset], "result_field")
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(pointer, field)
                .map_err(|error| error.to_string())?;
        }
        Ok(object.into())
    }
}
