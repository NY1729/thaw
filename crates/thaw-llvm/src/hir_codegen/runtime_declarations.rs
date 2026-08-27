impl<'ctx> HirCompiler<'ctx> {
    /// Declares libc's `puts`/`printf` (the Phase 0 `console.log` bootstrap)
    /// and thaw-arena's `thaw_arena_alloc` (backing Phase 1 arrays).
    fn declare_runtime_builtins(&self) {
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        let puts_type = i32_type.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("puts", puts_type, Some(Linkage::External));

        let printf_type = i32_type.fn_type(&[i8_ptr.into()], true);
        self.module
            .add_function("printf", printf_type, Some(Linkage::External));
        let dprintf_type = i32_type.fn_type(&[i32_type.into(), i8_ptr.into()], true);
        self.module
            .add_function("dprintf", dprintf_type, Some(Linkage::External));
        let pow_type = self.context.f64_type().fn_type(
            &[
                self.context.f64_type().into(),
                self.context.f64_type().into(),
            ],
            false,
        );
        self.module
            .add_function("pow", pow_type, Some(Linkage::External));
        let unary_f64_type = self
            .context
            .f64_type()
            .fn_type(&[self.context.f64_type().into()], false);
        for name in [
            "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "cbrt", "acosh", "asinh",
            "atanh", "expm1", "log1p",
        ] {
            self.module
                .add_function(name, unary_f64_type, Some(Linkage::External));
        }
        for name in ["atan2", "hypot"] {
            self.module
                .add_function(name, pow_type, Some(Linkage::External));
        }
        for name in ["thaw_math_fround", "thaw_math_clz32"] {
            self.module
                .add_function(name, unary_f64_type, Some(Linkage::External));
        }
        self.module
            .add_function("thaw_math_imul", pow_type, Some(Linkage::External));
        let math_random_type = self.context.f64_type().fn_type(&[], false);
        self.module.add_function(
            "thaw_math_random",
            math_random_type,
            Some(Linkage::External),
        );
        for name in [
            "llvm.fabs.f64",
            "llvm.floor.f64",
            "llvm.ceil.f64",
            "llvm.trunc.f64",
            "llvm.sqrt.f64",
            "llvm.exp.f64",
            "llvm.log.f64",
            "llvm.log2.f64",
            "llvm.log10.f64",
            "llvm.sin.f64",
            "llvm.cos.f64",
        ] {
            self.module
                .add_function(name, unary_f64_type, Some(Linkage::External));
        }
        let binary_f64_type = self.context.f64_type().fn_type(
            &[
                self.context.f64_type().into(),
                self.context.f64_type().into(),
            ],
            false,
        );
        for name in ["llvm.minimum.f64", "llvm.maximum.f64"] {
            self.module
                .add_function(name, binary_f64_type, Some(Linkage::External));
        }

        let arena_alloc_type = i8_ptr.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function(
            "thaw_arena_alloc",
            arena_alloc_type,
            Some(Linkage::External),
        );

        let strlen_type = i64_type.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("strlen", strlen_type, Some(Linkage::External));
        let strcmp_type = i32_type.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module
            .add_function("strcmp", strcmp_type, Some(Linkage::External));
        self.module
            .add_function("thaw_string_compare", strcmp_type, Some(Linkage::External));
        let memcpy_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), i64_type.into()], false);
        self.module
            .add_function("memcpy", memcpy_type, Some(Linkage::External));

        let getenv_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("getenv", getenv_type, Some(Linkage::External));

        // Lambda captures stdout via a pipe, not a TTY, so libc's stdio
        // fully-buffers it by default -- output could sit in the buffer
        // and never reach CloudWatch if the process is frozen/killed
        // between invocations. `console.log` flushes after every call to
        // avoid that (see `compile_console_log`).
        let fflush_type = i32_type.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("fflush", fflush_type, Some(Linkage::External));

        let run_http_servers_type = self.context.void_type().fn_type(&[], false);
        self.module.add_function(
            "thaw_http_run_servers",
            run_http_servers_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_http_take_unhandled_error",
            self.context.i8_type().fn_type(&[], false),
            Some(Linkage::External),
        );

        // thaw-std: fetch + JSON (see docs/design/async-await.md for why
        // `fetch` is a plain blocking call under the hood).
        let f64_type = self.context.f64_type();

        let fetch_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("thaw_fetch_get", fetch_type, Some(Linkage::External));

        let json_parse_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("thaw_json_parse", json_parse_type, Some(Linkage::External));

        let json_stringify_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_stringify",
            json_stringify_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_stringify_number_space",
            i8_ptr.fn_type(&[i8_ptr.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_stringify_string_space",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_stringify_keys",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_stringify_keys_number_space",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_stringify_keys_string_space",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );

        let json_get_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module
            .add_function("thaw_json_get", json_get_type, Some(Linkage::External));

        let json_index_type =
            i8_ptr.fn_type(&[i8_ptr.into(), f64_type.into(), i8_ptr.into()], false);
        self.module
            .add_function("thaw_json_index", json_index_type, Some(Linkage::External));
        let json_index_set_type = i8_ptr.fn_type(
            &[
                i8_ptr.into(),
                f64_type.into(),
                i8_ptr.into(),
                i8_ptr.into(),
            ],
            false,
        );
        self.module.add_function(
            "thaw_json_index_set",
            json_index_set_type,
            Some(Linkage::External),
        );

        let json_as_number_type = f64_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_as_number",
            json_as_number_type,
            Some(Linkage::External),
        );
        let number_to_string_type = i8_ptr.fn_type(&[f64_type.into()], false);
        self.module.add_function(
            "thaw_number_object_is",
            i8_type.fn_type(&[f64_type.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_number_to_string",
            number_to_string_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_number_to_fixed",
            i8_ptr.fn_type(&[f64_type.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_number_to_precision",
            i8_ptr.fn_type(&[f64_type.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        let string_to_number_type = f64_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_string_to_number",
            string_to_number_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_parse_float",
            string_to_number_type,
            Some(Linkage::External),
        );
        let parse_int_type = f64_type.fn_type(&[i8_ptr.into(), f64_type.into()], false);
        self.module
            .add_function("thaw_parse_int", parse_int_type, Some(Linkage::External));
        let array_to_string_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        for name in [
            "thaw_number_array_to_string",
            "thaw_string_array_to_string",
            "thaw_bool_array_to_string",
            "thaw_object_array_to_string",
        ] {
            self.module
                .add_function(name, array_to_string_type, Some(Linkage::External));
        }
        let array_join_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        for name in [
            "thaw_number_array_join",
            "thaw_string_array_join",
            "thaw_bool_array_join",
            "thaw_object_array_join",
        ] {
            self.module
                .add_function(name, array_join_type, Some(Linkage::External));
        }
        let array_reverse_type = i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into()], false);
        self.module.add_function(
            "thaw_array_reverse",
            array_reverse_type,
            Some(Linkage::External),
        );
        let array_copy_within_type = i8_ptr.fn_type(
            &[
                i8_ptr.into(),
                i64_type.into(),
                f64_type.into(),
                f64_type.into(),
                f64_type.into(),
            ],
            false,
        );
        self.module.add_function(
            "thaw_array_copy_within",
            array_copy_within_type,
            Some(Linkage::External),
        );
        for (name, value_type) in [
            ("thaw_number_array_fill", f64_type.into()),
            ("thaw_pointer_array_fill", i8_ptr.into()),
            ("thaw_bool_array_fill", i8_type.into()),
        ] {
            let ty = i8_ptr.fn_type(
                &[i8_ptr.into(), value_type, f64_type.into(), f64_type.into()],
                false,
            );
            self.module.add_function(name, ty, Some(Linkage::External));
        }
        let array_fill_type = i8_ptr.fn_type(
            &[
                i8_ptr.into(),
                i8_ptr.into(),
                i64_type.into(),
                f64_type.into(),
                f64_type.into(),
            ],
            false,
        );
        self.module
            .add_function("thaw_array_fill", array_fill_type, Some(Linkage::External));
        let array_slice_type = i8_ptr.fn_type(
            &[
                i8_ptr.into(),
                i64_type.into(),
                f64_type.into(),
                f64_type.into(),
            ],
            false,
        );
        self.module.add_function(
            "thaw_array_slice",
            array_slice_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_array_to_reversed",
            array_reverse_type,
            Some(Linkage::External),
        );
        let array_sort_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        for name in [
            "thaw_number_array_sort",
            "thaw_string_array_sort",
            "thaw_bool_array_sort",
            "thaw_object_array_sort",
            "thaw_number_array_to_sorted",
            "thaw_string_array_to_sorted",
            "thaw_bool_array_to_sorted",
            "thaw_object_array_to_sorted",
        ] {
            self.module
                .add_function(name, array_sort_type, Some(Linkage::External));
        }
        for (name, needle_type, return_type) in [
            (
                "thaw_number_array_index_of",
                f64_type.into(),
                f64_type.into(),
            ),
            (
                "thaw_number_array_includes",
                f64_type.into(),
                i8_type.into(),
            ),
            ("thaw_string_array_index_of", i8_ptr.into(), f64_type.into()),
            ("thaw_string_array_includes", i8_ptr.into(), i8_type.into()),
            ("thaw_bool_array_index_of", i8_type.into(), f64_type.into()),
            ("thaw_bool_array_includes", i8_type.into(), i8_type.into()),
            ("thaw_object_array_index_of", i8_ptr.into(), f64_type.into()),
            ("thaw_object_array_includes", i8_ptr.into(), i8_type.into()),
            (
                "thaw_number_array_last_index_of",
                f64_type.into(),
                f64_type.into(),
            ),
            (
                "thaw_string_array_last_index_of",
                i8_ptr.into(),
                f64_type.into(),
            ),
            (
                "thaw_bool_array_last_index_of",
                i8_type.into(),
                f64_type.into(),
            ),
            (
                "thaw_object_array_last_index_of",
                i8_ptr.into(),
                f64_type.into(),
            ),
        ] {
            let function_type = match return_type {
                BasicTypeEnum::FloatType(return_type) => {
                    return_type.fn_type(&[i8_ptr.into(), needle_type, f64_type.into()], false)
                }
                BasicTypeEnum::IntType(return_type) => {
                    return_type.fn_type(&[i8_ptr.into(), needle_type, f64_type.into()], false)
                }
                _ => unreachable!(),
            };
            self.module
                .add_function(name, function_type, Some(Linkage::External));
        }
        for name in [
            "thaw_string_includes",
            "thaw_string_starts_with",
            "thaw_string_ends_with",
        ] {
            let function_type =
                i8_type.fn_type(&[i8_ptr.into(), i8_ptr.into(), f64_type.into()], false);
            self.module
                .add_function(name, function_type, Some(Linkage::External));
        }
        let string_index_of_type =
            f64_type.fn_type(&[i8_ptr.into(), i8_ptr.into(), f64_type.into()], false);
        self.module.add_function(
            "thaw_string_index_of",
            string_index_of_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_string_last_index_of",
            string_index_of_type,
            Some(Linkage::External),
        );
        let string_transform_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        for name in [
            "thaw_string_trim",
            "thaw_string_trim_start",
            "thaw_string_trim_end",
            "thaw_string_to_lower_case",
            "thaw_string_to_upper_case",
        ] {
            self.module
                .add_function(name, string_transform_type, Some(Linkage::External));
        }
        self.module.add_function(
            "thaw_encode_uri_component",
            string_transform_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_decode_uri_component",
            string_transform_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_string_to_array",
            string_transform_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_string_repeat",
            i8_ptr.fn_type(&[i8_ptr.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        let string_pad_type =
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), f64_type.into()], false);
        for name in ["thaw_string_pad_start", "thaw_string_pad_end"] {
            self.module
                .add_function(name, string_pad_type, Some(Linkage::External));
        }
        let string_length_type = f64_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_string_length",
            string_length_type,
            Some(Linkage::External),
        );
        let string_char_code_type = f64_type.fn_type(&[i8_ptr.into(), f64_type.into()], false);
        self.module.add_function(
            "thaw_string_char_code_at",
            string_char_code_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_string_code_point_at",
            string_char_code_type,
            Some(Linkage::External),
        );

        let json_as_string_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_as_string",
            json_as_string_type,
            Some(Linkage::External),
        );

        // Returns i8 (0/1), not i1 -- see thaw-std's `thaw_json_as_bool`
        // doc comment on why it avoids relying on `bool`'s C ABI shape.
        let json_as_bool_type = self.context.i8_type().fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_as_bool",
            json_as_bool_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_array_new",
            i8_ptr.fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_null",
            i8_ptr.fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_is_array",
            self.context.i8_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_is_null",
            self.context.i8_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_keys",
            i8_ptr.fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_array_keys",
            i8_ptr.fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        for name in [
            "thaw_json_values",
            "thaw_json_number_values",
            "thaw_json_string_values",
            "thaw_json_bool_values",
            "thaw_json_entries",
            "thaw_json_number_entries",
            "thaw_json_string_entries",
            "thaw_json_bool_entries",
            "thaw_json_object_from_number_entries",
            "thaw_json_object_from_string_entries",
            "thaw_json_object_from_bool_entries",
            "thaw_json_object_from_json_entries",
        ] {
            self.module.add_function(
                name,
                i8_ptr.fn_type(&[i8_ptr.into()], false),
                Some(Linkage::External),
            );
        }
        self.module.add_function(
            "thaw_json_has_own",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_object_assign",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_object_is",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        for (name, other) in [
            ("thaw_json_object_is_number", f64_type.into()),
            ("thaw_json_object_is_string", i8_ptr.into()),
            (
                "thaw_json_object_is_bool",
                self.context.bool_type().into(),
            ),
        ] {
            self.module.add_function(
                name,
                self.context
                    .i8_type()
                    .fn_type(&[i8_ptr.into(), other], false),
                Some(Linkage::External),
            );
        }
        for (name, value_type) in [
            ("thaw_json_array_push_number", f64_type.into()),
            ("thaw_json_array_push_string", i8_ptr.into()),
            ("thaw_json_array_push_bool", self.context.i8_type().into()),
            ("thaw_json_array_push_json", i8_ptr.into()),
        ] {
            self.module.add_function(
                name,
                self.context
                    .void_type()
                    .fn_type(&[i8_ptr.into(), value_type], false),
                Some(Linkage::External),
            );
        }
        for name in ["thaw_json_from_number_array", "thaw_json_to_number_array"] {
            self.module.add_function(
                name,
                i8_ptr.fn_type(&[i8_ptr.into()], false),
                Some(Linkage::External),
            );
        }
        self.module.add_function(
            "thaw_json_array_length",
            self.context.i64_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_object_new",
            i8_ptr.fn_type(&[], false),
            Some(Linkage::External),
        );
        for (name, value_type) in [
            ("thaw_json_object_set_number", f64_type.into()),
            ("thaw_json_object_set_string", i8_ptr.into()),
            ("thaw_json_object_set_bool", self.context.i8_type().into()),
            ("thaw_json_object_set_json", i8_ptr.into()),
        ] {
            self.module.add_function(
                name,
                self.context
                    .void_type()
                    .fn_type(&[i8_ptr.into(), i8_ptr.into(), value_type], false),
                Some(Linkage::External),
            );
        }
        self.module.add_function(
            "thaw_json_object_delete",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );

        // thaw-quickjs: the QuickJS-NG fallback path (docs/design/bridge.md
        // section 7). Same i8-not-i1 reasoning as `thaw_json_as_bool`.
        let js_load_type = self.context.i8_type().fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("thaw_js_load", js_load_type, Some(Linkage::External));

        let js_call_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module
            .add_function("thaw_js_call", js_call_type, Some(Linkage::External));
        let result_type = self
            .context
            .struct_type(&[i8_ptr.into(), i8_ptr.into()], false);
        let js_call_result_type = result_type.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module.add_function(
            "thaw_js_call_result",
            js_call_result_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_get_global",
            self.context.i64_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_handle_to_string",
            i8_ptr.fn_type(&[self.context.i64_type().into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_result",
            result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_handle_result",
            self.context
                .struct_type(&[self.context.i64_type().into(), i8_ptr.into()], false)
                .fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_value_result",
            result_type.fn_type(
                &[
                    self.context.i64_type().into(),
                    self.context.i64_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_release_handle",
            self.context
                .i8_type()
                .fn_type(&[self.context.i64_type().into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_release_all_handles",
            self.context.i64_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        let handle_result_type = self
            .context
            .struct_type(&[self.context.i64_type().into(), i8_ptr.into()], false);
        self.module.add_function(
            "thaw_js_get_property_result",
            handle_result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_set_property_result",
            handle_result_type.fn_type(
                &[
                    self.context.i64_type().into(),
                    i8_ptr.into(),
                    self.context.i64_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_method_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_resolve_handle_result",
            result_type.fn_type(&[self.context.i64_type().into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_mixed_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_construct_handle_result",
            handle_result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module
            .add_function("thaw_napi_load", js_load_type, Some(Linkage::External));
        self.module.add_function(
            "thaw_napi_load_named",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_load_embedded_hex",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_result",
            js_call_result_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_typed_result",
            js_call_result_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_get_export",
            self.context.i64_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_construct_handle_result",
            handle_result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_construct_handle_typed_result",
            handle_result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_method_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_method_typed_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_get_property_result",
            result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_get_property_typed_result",
            result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_set_property_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_set_property_typed_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_method_with_callback_result",
            result_type.fn_type(
                &[
                    self.context.i64_type().into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    self.context.i8_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_with_callback_result",
            result_type.fn_type(
                &[i8_ptr.into(), i8_ptr.into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_run_async_work",
            self.context.i64_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_poll_async_work",
            self.context.i64_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_unload_all",
            self.context.i8_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_take_fatal_exception",
            self.context.i8_type().fn_type(&[], false),
            Some(Linkage::External),
        );

        let sleep_type = i8_ptr.fn_type(&[i64_type.into()], false);
        self.module
            .add_function("thaw_sleep_ms", sleep_type, Some(Linkage::External));
        self.module.add_function(
            "thaw_http_get_async",
            i8_ptr.fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        let run_until_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_runtime_run_until_resolved",
            run_until_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_runtime_drain_detached",
            i64_type.fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_new",
            i8_ptr.fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_subscribe",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_resolve",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_reject",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_state",
            self.context.i8_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_destroy",
            self.context.void_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_chain",
            i8_ptr.fn_type(
                &[
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    self.context.i8_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_adopt",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_finally",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_finally_adopt",
            self.context.i8_type().fn_type(
                &[
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    self.context.i8_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_all_slots",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_all_typed",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_race",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_any",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_all_settled",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
    }
}
