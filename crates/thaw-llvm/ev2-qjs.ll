; ModuleID = 'scratch'
source_filename = "scratch"

@__thaw_stack_limit = internal global i64 0
@__thaw_pending_exception = internal global ptr null
@__thaw_pending_rejection = internal global ptr null
@__thaw_active_rejection_text = internal global ptr null
@__thaw_pending_exception_native_text = internal global ptr null
@__thaw_pending_exception_object = internal global ptr null
@__thaw_pending_exception_native = internal global ptr null
@__thaw_pending_exception_aggregate_errors = internal global ptr null
@__thaw_pending_exception_value_tag = internal global i64 0
@__thaw_pending_exception_f64 = internal global double 0.000000e+00
@__thaw_pending_exception_i64 = internal global i64 0
@__thaw_pending_exception_bool = internal global i1 false
@synthetic_error_message = private unnamed_addr constant [28 x i8] c"Cannot allocate async frame\00", align 1
@synthetic_error_name = private unnamed_addr constant [10 x i8] c"TypeError\00", align 1
@synthetic_error_message.1 = private unnamed_addr constant [33 x i8] c"Cannot allocate async completion\00", align 1
@synthetic_error_message.2 = private unnamed_addr constant [45 x i8] c"Unable to store native Promise in arena slot\00", align 1
@strlit = private unnamed_addr constant [6 x i8] c"Error\00", align 1
@strlit.3 = private unnamed_addr constant [2 x i8] c"x\00", align 1
@strlit.4 = private unnamed_addr constant [45 x i8] c"\01RangeError\01Maximum call stack size exceeded\00", align 1
@strlit.5 = private unnamed_addr constant [6 x i8] c"value\00", align 1
@napi_undefined_key = private unnamed_addr constant [24 x i8] c"$__thaw_napi_undefined$\00", align 1
@synthetic_error_message.6 = private unnamed_addr constant [24 x i8] c"Value is not a function\00", align 1
@strlit.7 = private unnamed_addr constant [29 x i8] c"__thaw_is_null_dynamic_value\00", align 1
@synthetic_error_message.8 = private unnamed_addr constant [32 x i8] c"Invalid JavaScript result graph\00", align 1
@strlit.9 = private unnamed_addr constant [34 x i8] c"__thaw_is_undefined_dynamic_value\00", align 1
@invalid_callback_graph_message = private unnamed_addr constant [41 x i8] c"\01TypeError\01Invalid native callback graph\00", align 1
@cyclic_callback_type_error = private unnamed_addr constant [49 x i8] c"\01TypeError\01Converting circular structure to JSON\00", align 1
@native_callback_undefined_name = private unnamed_addr constant [10 x i8] c"undefined\00", align 1
@strlit.10 = private unnamed_addr constant [11 x i8] c"setTimeout\00", align 1
@synthetic_error_message.11 = private unnamed_addr constant [35 x i8] c"Promise executor is not a function\00", align 1
@synthetic_error_message.12 = private unnamed_addr constant [26 x i8] c"Promise allocation failed\00", align 1
@synthetic_error_message.13 = private unnamed_addr constant [35 x i8] c"Promise resolver allocation failed\00", align 1
@synthetic_error_message.14 = private unnamed_addr constant [42 x i8] c"Promise resolve closure allocation failed\00", align 1
@synthetic_error_message.15 = private unnamed_addr constant [41 x i8] c"Promise reject closure allocation failed\00", align 1
@strlit.16 = private unnamed_addr constant [6 x i8] c"after\00", align 1
@stack_fn_name = private unnamed_addr constant [2 x i8] c"f\00", align 1
@stack_fn_file = private unnamed_addr constant [9 x i8] c"input.ts\00", align 1
@stack_fn_name.17 = private unnamed_addr constant [5 x i8] c"main\00", align 1
@thaw_stack_table = internal constant [4 x { ptr, ptr, ptr, i32, i32 }] [{ ptr, ptr, ptr, i32, i32 } { ptr @f, ptr @stack_fn_name, ptr @stack_fn_file, i32 1, i32 1 }, { ptr, ptr, ptr, i32, i32 } { ptr @thaw_user_main, ptr @stack_fn_name.17, ptr @stack_fn_file, i32 2, i32 1 }, { ptr, ptr, ptr, i32, i32 } { ptr @f.resume, ptr @stack_fn_name, ptr @stack_fn_file, i32 1, i32 1 }, { ptr, ptr, ptr, i32, i32 } { ptr @thaw_user_main.resume, ptr @stack_fn_name.17, ptr @stack_fn_file, i32 2, i32 1 }]
@stack_file = private unnamed_addr constant [8 x i8] c"scratch\00", align 1
@__start_thaw_user_fns = external global i8
@__stop_thaw_user_fns = external global i8

declare i32 @puts(ptr)

declare i32 @printf(ptr, ...)

declare i32 @dprintf(i32, ptr, ...)

declare double @pow(double, double)

declare double @tan(double)

declare double @asin(double)

declare double @acos(double)

declare double @atan(double)

declare double @sinh(double)

declare double @cosh(double)

declare double @tanh(double)

declare double @cbrt(double)

declare double @acosh(double)

declare double @asinh(double)

declare double @atanh(double)

declare double @expm1(double)

declare double @log1p(double)

declare ptr @thaw_error_suppress(ptr, ptr, ptr)

declare ptr @thaw_object_class_name(ptr)

declare ptr @thaw_i64_to_string(i64)

declare ptr @thaw_i64_to_bigint_string(i64)

declare i64 @thaw_bigint_from_f64(double)

declare i64 @thaw_bigint_from_i64(i64)

declare i64 @thaw_bigint_from_u64(i64)

declare i64 @thaw_bigint_to_i64_wrapping(i64)

declare double @thaw_bigint_to_f64(i64)

declare i64 @thaw_bigint_neg(i64)

declare i64 @thaw_bigint_not(i64)

declare i1 @thaw_bigint_is_zero(i64)

declare i1 @thaw_bigint_parse_ok(ptr)

declare ptr @thaw_bigint_parse_error_message(ptr)

declare i32 @thaw_bigint_cmp(i64, i64)

declare i32 @thaw_bigint_cmp_f64(i64, double)

declare i32 @thaw_bigint_binop_error(i32, i64, i64)

declare ptr @thaw_json_as_bigint_decimal(ptr)

declare ptr @thaw_json_receiver_bigint_decimal(ptr)

declare i64 @thaw_bigint_add(i64, i64)

declare i64 @thaw_bigint_sub(i64, i64)

declare i64 @thaw_bigint_mul(i64, i64)

declare i64 @thaw_bigint_div(i64, i64)

declare i64 @thaw_bigint_mod(i64, i64)

declare i64 @thaw_bigint_and(i64, i64)

declare i64 @thaw_bigint_or(i64, i64)

declare i64 @thaw_bigint_xor(i64, i64)

declare i64 @thaw_bigint_shl(i64, i64)

declare i64 @thaw_bigint_shr(i64, i64)

declare i64 @thaw_i64_from_string(ptr)

declare i64 @thaw_i64_as_int_n(i64, double)

declare i64 @thaw_i64_as_uint_n(i64, double)

declare ptr @thaw_i64_to_radix_string(i64, double)

declare i8 @thaw_math_sum_precise(ptr, ptr, ptr)

declare ptr @thaw_symbol_new(ptr)

declare ptr @thaw_symbol_to_string(ptr)

declare ptr @thaw_symbol_key(ptr)

declare ptr @thaw_symbol_for(ptr)

declare ptr @thaw_symbol_key_for(ptr)

declare ptr @thaw_symbol_description(ptr)

declare double @atan2(double, double)

declare double @hypot(double, double)

declare double @thaw_math_fround(double)

declare double @thaw_math_clz32(double)

declare double @thaw_math_imul(double, double)

declare double @thaw_math_random()

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.fabs.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.floor.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.ceil.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.trunc.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.sqrt.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.exp.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.log.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.log2.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.log10.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.sin.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.cos.f64(double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.minimum.f64(double, double) #0

; Function Attrs: nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.maximum.f64(double, double) #0

declare ptr @thaw_arena_alloc(i64, i64)

declare ptr @thaw_non_arrow_receiver_new(i8, i64)

declare i64 @thaw_string_byte_length(ptr)

declare ptr @thaw_string_append_owned(ptr, ptr, ptr)

declare ptr @thaw_string_register(ptr, i64)

declare ptr @thaw_string_register_literal(ptr, i64)

declare void @thaw_console_write(ptr, i32, i1)

declare void @thaw_console_group(i32)

declare void @thaw_console_label(i32, ptr)

declare ptr @thaw_console_table_text(ptr, ptr)

declare ptr @thaw_console_dir_text(ptr, ptr)

declare i64 @strlen(ptr)

declare i32 @strcmp(ptr, ptr)

declare i32 @thaw_string_compare(ptr, ptr)

declare i32 @thaw_string_strict_equal(ptr, ptr)

declare i32 @thaw_intl_compare(ptr, ptr, ptr, ptr)

declare ptr @thaw_intl_number(double, ptr, ptr)

declare ptr @thaw_intl_date(double, ptr, ptr, double)

declare i8 @thaw_json_strict_equal(ptr, ptr)

declare ptr @memcpy(ptr, ptr, i64)

declare ptr @getenv(ptr)

declare void @thaw_runtime_report_uncaught(ptr)

declare ptr @thaw_runtime_exception_report_text(ptr, ptr, i64, double, i64, i1)

declare i8 @thaw_js_emit_uncaught(ptr)

declare i8 @thaw_js_emit_unhandled_rejection(ptr)

declare void @thaw_js_emit_rejection_handled()

declare { i64, ptr } @thaw_js_emit_uncaught_result(ptr)

declare { i64, ptr } @thaw_js_emit_unhandled_rejection_result(ptr)

declare { i64, ptr } @thaw_js_emit_rejection_handled_result()

declare i32 @fflush(ptr)

declare void @thaw_http_run_servers()

declare i8 @thaw_http_take_unhandled_error()

declare ptr @thaw_fetch_get(ptr)

declare ptr @thaw_json_parse(ptr)

declare ptr @thaw_json_parse_error_property(ptr)

declare ptr @thaw_json_parse_error_message(ptr)

declare i8 @thaw_json_take_parse_error()

declare void @thaw_json_destroy(ptr)

declare void @thaw_json_typed_decode_scope_begin()

declare void @thaw_json_typed_decode_scope_own(ptr)

declare void @thaw_json_typed_decode_scope_end(i8)

declare i64 @thaw_json_handle_id(ptr)

declare i64 @thaw_json_callback_origin_acquire(ptr)

declare ptr @thaw_closure_alias_register(ptr, ptr)

declare ptr @thaw_closure_alias_resolve(ptr)

declare i8 @thaw_json_register_callback_origin(ptr, i64)

declare i64 @thaw_json_borrowed_handle_id(ptr)

declare ptr @thaw_json_from_borrowed_handle(i64)

declare ptr @thaw_json_host_from_borrowed_handle(i64)

declare ptr @thaw_json_track_arena_owned_root(ptr)

declare { ptr, i64, ptr } @thaw_json_store_arena_binding(ptr, ptr, ptr, i8)

declare { ptr, i64, ptr } @thaw_json_store_global_binding(i64, ptr, i8)

declare i64 @thaw_json_register_global_binding(ptr)

declare i8 @thaw_json_release_global_binding(i64)

declare i8 @thaw_arena_unregister_root(ptr)

declare i64 @thaw_json_root_pin(ptr)

declare i8 @thaw_json_root_unpin(i64)

declare ptr @thaw_exception_native_provenance_new(i64, ptr, ptr)

declare i64 @thaw_exception_native_tag(ptr)

declare ptr @thaw_exception_native_owner(ptr, i64, ptr)

declare i8 @thaw_str_pointer_same(ptr, ptr)

declare i8 @thaw_exception_native_same(ptr, ptr)

declare void @thaw_cstring_destroy(ptr)

declare i8 @thaw_json_take_stringify_error()

declare ptr @thaw_json_stringify_error_message()

declare ptr @thaw_json_brand_wrapper(ptr)

declare ptr @thaw_json_brand_handle_placeholder(ptr)

declare void @thaw_json_adopt_native_date(ptr, ptr, i1)

declare ptr @thaw_json_brand_wrapper_if(ptr, i1)

declare ptr @thaw_json_graph_encode(ptr)

declare ptr @thaw_json_graph_decode(ptr)

declare void @thaw_json_discard_graph_wire(ptr)

declare void @thaw_json_register_host_operations(ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr)

declare void @thaw_json_set_js_engine(i8)

declare void @thaw_json_register_error_factory(ptr, ptr, ptr)

declare ptr @thaw_json_take_host_error()

declare i8 @thaw_json_take_graph_error()

declare ptr @thaw_json_stringify(ptr)

declare ptr @thaw_json_stringify_public(ptr)

declare ptr @thaw_json_stringify_tjs(ptr)

declare ptr @thaw_json_encode_public(ptr)

declare ptr @thaw_json_callback_error(ptr)

declare ptr @thaw_json_callback_error_tuple(ptr, i64, double, i64, i8, ptr, ptr, ptr, ptr)

declare i8 @thaw_exception_token_is_live(i64)

declare i64 @thaw_exception_native_token(ptr)

declare i64 @thaw_js_exception_native_token(i64)

declare i8 @thaw_exception_native_snapshot(i64, ptr)

declare ptr @thaw_json_stringify_number_space(ptr, double)

declare ptr @thaw_json_stringify_string_space(ptr, ptr)

declare ptr @thaw_json_stringify_keys(ptr, ptr, ptr)

declare ptr @thaw_json_stringify_keys_number_space(ptr, ptr, ptr, double)

declare ptr @thaw_json_stringify_keys_string_space(ptr, ptr, ptr, ptr)

declare ptr @thaw_json_get(ptr, ptr)

declare ptr @thaw_json_take(ptr, ptr)

declare ptr @thaw_json_get_mut(ptr, ptr)

declare ptr @thaw_json_index_get_mut(ptr, double)

declare ptr @thaw_json_index(ptr, double, ptr)

declare ptr @thaw_json_index_set(ptr, double, ptr, ptr)

declare double @thaw_json_as_number(ptr)

declare i64 @thaw_json_as_bigint_i64(ptr)

declare double @thaw_jit_dictionary_get(i8, ptr, ptr, ptr, ptr)

declare double @thaw_jit_dictionary_mutate(i8, ptr, ptr, double, ptr)

declare double @thaw_jit_dictionary_query(i8, ptr, ptr, ptr)

declare i8 @thaw_number_object_is(double, double)

declare ptr @thaw_number_to_string(double)

declare ptr @thaw_number_to_console_string(double)

declare ptr @thaw_string_from_char_code(double)

declare ptr @thaw_string_from_code_point(double)

declare ptr @thaw_number_to_fixed(double, double)

declare ptr @thaw_number_to_precision(double, double)

declare ptr @thaw_number_to_exponential(double, double)

declare ptr @thaw_number_to_radix_string(double, double)

declare ptr @thaw_jit_format_number(i8, double, double)

declare double @thaw_jit_array_search(i8, ptr, double, double)

declare ptr @thaw_jit_array_format(i8, ptr, ptr)

declare ptr @thaw_jit_array_to_sorted(i8, ptr)

declare ptr @thaw_jit_array_sort(i8, ptr)

declare ptr @thaw_jit_array_fill(i8, ptr, double, double, double)

declare double @thaw_jit_array_push(i8, ptr, double)

declare double @thaw_jit_array_unshift(i8, ptr, double)

declare i8 @thaw_jit_array_remove(i8, ptr, ptr)

declare ptr @thaw_jit_array_splice(ptr, double, double, ptr)

declare i8 @thaw_jit_array_set(i8, ptr, double, double)

declare ptr @thaw_array_concat(ptr, ptr, i64)

declare ptr @thaw_jit_array_append(i8, ptr, double)

declare ptr @thaw_jit_array_with(i8, ptr, double, double)

declare double @thaw_string_to_number(ptr)

declare double @thaw_parse_float(ptr)

declare double @thaw_parse_int(ptr, double)

declare ptr @thaw_number_array_to_string(ptr, ptr)

declare ptr @thaw_string_array_to_string(ptr, ptr)

declare ptr @thaw_bool_array_to_string(ptr, ptr)

declare ptr @thaw_object_array_to_string(ptr, ptr)

declare ptr @thaw_number_array_join(ptr, ptr, ptr)

declare ptr @thaw_bigint_array_join(ptr, ptr, ptr)

declare ptr @thaw_string_array_join(ptr, ptr, ptr)

declare ptr @thaw_bool_array_join(ptr, ptr, ptr)

declare ptr @thaw_object_array_join(ptr, ptr, ptr)

declare ptr @thaw_tagged_array_join(ptr, ptr, ptr, i8, i8, ptr)

declare ptr @thaw_array_reverse(ptr, i64)

declare ptr @thaw_array_presence_reverse(ptr)

declare i8 @thaw_array_has_property(ptr, ptr, ptr, i8)

declare ptr @thaw_array_presence_densify(ptr)

declare ptr @thaw_array_presence_mapped(ptr)

declare i64 @thaw_array_presence_compact(ptr, ptr, i64)

declare ptr @thaw_array_presence_tagged_sort(ptr, ptr, i8, i8)

declare ptr @thaw_array_copy_within(ptr, i64, double, double, double)

declare ptr @thaw_array_presence_copy_within(ptr, double, double, double)

declare ptr @thaw_array_presence_fill(ptr, i64, double, double, i8)

declare i8 @thaw_array_delete_property(ptr, ptr)

declare void @thaw_array_presence_mark(ptr, i64)

declare ptr @thaw_array_ensure_index(ptr, i64, double)

declare ptr @thaw_array_read_ptr(ptr, i64, double)

declare ptr @thaw_array_read_ptr_num(ptr, i64, double)

declare ptr @thaw_any_array_flat(ptr, ptr, double)

declare ptr @thaw_array_resize(ptr, i64, double)

declare ptr @thaw_array_presence_set_state(ptr, i64, i64, i8)

declare ptr @thaw_array_presence_extend(ptr, i64, i64, i8, ptr)

declare ptr @thaw_array_presence_remove(ptr, i64, i8)

declare ptr @thaw_number_array_fill(ptr, double, double, double)

declare ptr @thaw_pointer_array_fill(ptr, ptr, double, double)

declare ptr @thaw_bool_array_fill(ptr, i8, double, double)

declare i64 @thaw_bigint_pow(i64, i64)

declare ptr @thaw_array_fill(ptr, ptr, i64, double, double)

declare ptr @thaw_array_slice(ptr, i64, double, double)

declare ptr @thaw_array_presence_slice(ptr, i64, double, double)

declare ptr @thaw_array_presence_rest(ptr, i64, double, double)

declare ptr @thaw_array_to_reversed(ptr, i64)

declare ptr @thaw_array_push_values(ptr, i64, ptr, i64)

declare ptr @thaw_array_unshift_values(ptr, i64, ptr, i64)

declare ptr @thaw_array_pop(ptr, i64, ptr)

declare ptr @thaw_array_shift(ptr, i64, ptr)

declare ptr @thaw_array_splice(ptr, i64, double, double, ptr, i64, ptr)

declare ptr @thaw_array_presence_splice(ptr, i64, double, double, i64, ptr, ptr)

declare ptr @thaw_number_array_sort(ptr)

declare ptr @thaw_bigint_array_sort(ptr)

declare ptr @thaw_bigint_array_to_sorted(ptr)

declare ptr @thaw_string_array_sort(ptr)

declare ptr @thaw_bool_array_sort(ptr)

declare ptr @thaw_object_array_sort(ptr)

declare ptr @thaw_number_array_to_sorted(ptr)

declare ptr @thaw_string_array_to_sorted(ptr)

declare ptr @thaw_bool_array_to_sorted(ptr)

declare ptr @thaw_object_array_to_sorted(ptr)

declare double @thaw_number_array_comparator_sort(ptr, double, double)

declare double @thaw_array_undefined_index_of(ptr, ptr, double, i8, i8, i8, i8)

declare double @thaw_tagged_array_index_of(ptr, ptr, ptr, double, i8, i8, i8, i8, i8)

declare double @thaw_number_array_index_of(ptr, ptr, double, double)

declare i8 @thaw_number_array_includes(ptr, ptr, double, double)

declare double @thaw_string_array_index_of(ptr, ptr, ptr, double)

declare i8 @thaw_string_array_includes(ptr, ptr, ptr, double)

declare double @thaw_bool_array_index_of(ptr, ptr, i8, double)

declare i8 @thaw_bool_array_includes(ptr, ptr, i8, double)

declare double @thaw_bigint_array_index_of(ptr, ptr, i64, double)

declare i8 @thaw_bigint_array_includes(ptr, ptr, i64, double)

declare double @thaw_bigint_array_last_index_of(ptr, ptr, i64, double)

declare double @thaw_object_array_index_of(ptr, ptr, ptr, double)

declare i8 @thaw_object_array_includes(ptr, ptr, ptr, double)

declare double @thaw_any_array_index_of(ptr, ptr, ptr, double)

declare i8 @thaw_any_array_includes(ptr, ptr, ptr, double)

declare double @thaw_number_array_last_index_of(ptr, ptr, double, double)

declare double @thaw_string_array_last_index_of(ptr, ptr, ptr, double)

declare double @thaw_bool_array_last_index_of(ptr, ptr, i8, double)

declare double @thaw_object_array_last_index_of(ptr, ptr, ptr, double)

declare double @thaw_any_array_last_index_of(ptr, ptr, ptr, double)

declare i8 @thaw_string_includes(ptr, ptr, double)

declare i8 @thaw_string_starts_with(ptr, ptr, double)

declare i8 @thaw_string_ends_with(ptr, ptr, double)

declare double @thaw_string_index_of(ptr, ptr, double)

declare double @thaw_string_last_index_of(ptr, ptr, double)

declare ptr @thaw_string_slice(ptr, double, double)

declare ptr @thaw_string_substring(ptr, double, double)

declare ptr @thaw_string_substr(ptr, double, double)

declare ptr @thaw_string_trim(ptr)

declare ptr @thaw_string_trim_start(ptr)

declare ptr @thaw_string_trim_end(ptr)

declare ptr @thaw_string_to_lower_case(ptr)

declare ptr @thaw_string_to_upper_case(ptr)

declare ptr @thaw_string_to_well_formed(ptr)

declare ptr @thaw_string_to_display(ptr)

declare ptr @thaw_atob(ptr)

declare ptr @thaw_btoa(ptr)

declare ptr @thaw_escape(ptr)

declare ptr @thaw_unescape(ptr)

declare i1 @thaw_string_is_well_formed(ptr)

declare ptr @thaw_string_to_locale_lower_case(ptr, ptr)

declare ptr @thaw_string_to_locale_upper_case(ptr, ptr)

declare ptr @thaw_encode_uri_component(ptr)

declare ptr @thaw_encode_uri(ptr)

declare ptr @thaw_decode_uri_component(ptr)

declare ptr @thaw_decode_uri(ptr)

declare ptr @thaw_string_normalize(ptr, ptr)

declare ptr @thaw_string_to_array(ptr)

declare ptr @thaw_bytes_to_string(ptr, ptr)

declare ptr @thaw_bytes_from_string(ptr, ptr)

declare ptr @thaw_bytes_alloc(double)

declare ptr @thaw_bytes_alloc_fill(double, ptr)

declare ptr @thaw_bytes_set_from_string(ptr, ptr, ptr)

declare ptr @thaw_bytes_from_array(ptr)

declare ptr @thaw_bytes_concat(ptr, double)

declare double @thaw_bytes_byte_length(ptr, ptr)

declare i8 @thaw_bytes_equals(ptr, ptr)

declare double @thaw_bytes_read(ptr, double, double, double, double)

declare double @thaw_bytes_write(ptr, double, double, double, double, double)

declare i64 @thaw_bytes_read_i64(ptr, double, double)

declare double @thaw_bytes_write_i64(ptr, double, i64, double)

declare double @thaw_bytes_copy(ptr, ptr, double, double, double)

declare double @thaw_bytes_byte_offset(ptr)

declare double @thaw_bytes_store_length(ptr)

declare ptr @thaw_bytes_subarray(ptr, double, double)

declare ptr @thaw_bytes_slice(ptr, double, double)

declare ptr @thaw_bytes_slice_copy(ptr, double, double)

declare void @thaw_bytes_reorder(ptr, double)

declare void @thaw_bytes_fill(ptr, ptr, double, double)

declare ptr @thaw_arraybuffer_transfer(ptr, double)

declare ptr @thaw_arraybuffer_alloc(double)

declare ptr @thaw_dataview_new(ptr, double, double)

declare double @thaw_dataview_get(ptr, double, double, double)

declare void @thaw_dataview_set(ptr, double, double, double, double)

declare double @thaw_bytes_console_info(ptr, double)

declare ptr @thaw_bytes_console_target(ptr)

declare ptr @thaw_bytes_from_buffer_like(ptr, double, double, double)

declare ptr @thaw_bytes_to_array(ptr)

declare ptr @thaw_bytes_widen_list(ptr)

declare ptr @thaw_bytes_narrow_list(ptr)

declare ptr @thaw_bytes_as_u8(ptr)

declare ptr @thaw_bytes_clone(ptr)

declare ptr @thaw_bytes_buffer(ptr)

declare void @thaw_bytes_store_array(ptr, ptr)

declare double @thaw_bytes_index_of(ptr, ptr, double, double)

declare ptr @thaw_string_split(ptr, ptr, double)

declare ptr @thaw_string_replace(ptr, ptr, ptr)

declare ptr @thaw_string_replace_all(ptr, ptr, ptr)

declare i8 @thaw_regex_test(ptr, ptr, ptr)

declare double @thaw_regex_search(ptr, ptr, ptr)

declare ptr @thaw_regex_match(ptr, ptr, ptr, double)

declare ptr @thaw_regex_match_all(ptr, ptr, ptr, double)

declare ptr @thaw_regex_split(ptr, ptr, ptr, double)

declare ptr @thaw_regex_exec(ptr, ptr, ptr, double)

declare ptr @thaw_regex_check(ptr, ptr)

declare ptr @thaw_regex_exec_groups(ptr)

declare double @thaw_regex_exec_index(ptr)

declare ptr @thaw_regex_console_wrap(ptr, ptr)

declare ptr @thaw_regex_exec_input(ptr)

declare void @thaw_template_strings_register(ptr, ptr)

declare ptr @thaw_template_site(double, ptr, ptr)

declare ptr @thaw_template_strings_raw(ptr)

declare double @thaw_regex_exec_advance(ptr, ptr, ptr, double)

declare ptr @thaw_regex_replace(ptr, ptr, ptr, ptr)

declare ptr @thaw_regex_replace_all(ptr, ptr, ptr, ptr)

declare double @thaw_date_now()

declare double @thaw_performance_now()

declare double @thaw_process_pid()

declare double @thaw_process_ppid()

declare double @thaw_date_get_full_year(double)

declare double @thaw_date_get_month(double)

declare double @thaw_date_get_date(double)

declare double @thaw_date_get_day(double)

declare double @thaw_date_get_hours(double)

declare double @thaw_date_get_minutes(double)

declare double @thaw_date_get_seconds(double)

declare double @thaw_date_get_milliseconds(double)

declare double @thaw_date_get_local_full_year(double)

declare double @thaw_date_get_local_month(double)

declare double @thaw_date_get_local_date(double)

declare double @thaw_date_get_local_day(double)

declare double @thaw_date_get_local_hours(double)

declare double @thaw_date_get_local_minutes(double)

declare double @thaw_date_get_local_seconds(double)

declare double @thaw_date_get_local_milliseconds(double)

declare double @thaw_date_get_month_for_full_year(double)

declare double @thaw_date_get_date_for_full_year(double)

declare double @thaw_date_get_local_month_for_full_year(double)

declare double @thaw_date_get_local_date_for_full_year(double)

declare double @thaw_date_time_clip(double)

declare double @thaw_date_get_timezone_offset(double)

declare ptr @thaw_date_to_iso_string(double)

declare ptr @thaw_date_to_date_string(double)

declare ptr @thaw_date_to_time_string(double)

declare ptr @thaw_date_to_string(double)

declare ptr @thaw_date_to_utc_string(double)

declare double @thaw_bigint_decimal_cmp(ptr, ptr)

declare ptr @thaw_bigint_decimal_add(ptr, ptr)

declare ptr @thaw_bigint_decimal_sub(ptr, ptr)

declare ptr @thaw_bigint_decimal_mul(ptr, ptr)

declare ptr @thaw_bigint_decimal_div(ptr, ptr)

declare ptr @thaw_bigint_decimal_mod(ptr, ptr)

declare ptr @thaw_bigint_decimal_and(ptr, ptr)

declare ptr @thaw_bigint_decimal_or(ptr, ptr)

declare ptr @thaw_bigint_decimal_xor(ptr, ptr)

declare ptr @thaw_bigint_decimal_shl(ptr, ptr)

declare ptr @thaw_bigint_decimal_shr(ptr, ptr)

declare double @thaw_temporal_now()

declare double @thaw_temporal_now_nanos()

declare ptr @thaw_temporal_time_zone_id()

declare double @thaw_temporal_instant_from_string(ptr)

declare double @thaw_temporal_instant_nanos_from_string(ptr)

declare double @thaw_temporal_plain_date_time_from_string(ptr)

declare double @thaw_temporal_plain_date_time_nanos_from_string(ptr)

declare double @thaw_temporal_plain_time_from_string(ptr)

declare double @thaw_temporal_plain_time_nanos_from_string(ptr)

declare double @thaw_temporal_plain_month_day_from_string(ptr)

declare double @thaw_temporal_duration_nanos_from_string(ptr)

declare ptr @thaw_temporal_instant_to_string(double, double)

declare ptr @thaw_temporal_plain_date_to_string(double, double)

declare ptr @thaw_temporal_plain_date_time_to_string(double, double)

declare ptr @thaw_temporal_plain_time_to_string(double, double)

declare ptr @thaw_temporal_plain_year_month_to_string(double, double)

declare ptr @thaw_temporal_plain_month_day_to_string(double, double)

declare ptr @thaw_temporal_duration_to_string(double, double)

declare ptr @thaw_temporal_epoch_nanoseconds(double, double)

declare double @thaw_temporal_shift(double, double)

declare double @thaw_temporal_duration_component(double, double)

declare double @thaw_temporal_compare(double, double, double, double)

declare double @thaw_temporal_round(double, double, double)

declare i1 @thaw_temporal_zone_valid(ptr)

declare double @thaw_temporal_zoned_from_string(ptr)

declare double @thaw_temporal_zoned_nanos_from_string(ptr)

declare ptr @thaw_temporal_zoned_zone_from_string(ptr)

declare double @thaw_temporal_zoned_start_of_day(double, double, ptr)

declare double @thaw_temporal_zoned_hours_in_day(double, double, ptr)

declare double @thaw_temporal_zoned_transition(double, double, ptr, double, double)

declare double @thaw_temporal_plain_to_zoned(double, double, ptr, double)

declare double @thaw_temporal_with_fields(double, double, double, double, double, double, double, double, double, double, double, double, double, double)

declare ptr @thaw_temporal_zoned_to_string(double, double, ptr)

declare ptr @thaw_temporal_zoned_offset(double, double, ptr)

declare double @thaw_temporal_zoned_field(double, double, ptr, double)

declare double @thaw_temporal_zoned_plain_timestamp(double, double, ptr, double)

declare double @thaw_temporal_plain_date_field(double, double)

declare i1 @thaw_temporal_calendar_valid(ptr)

declare ptr @thaw_temporal_calendar_from_string(ptr)

declare double @thaw_temporal_calendar_field(double, ptr, double)

declare ptr @thaw_temporal_date_difference(double, double, ptr)

declare ptr @thaw_temporal_duration_balance(double, double, ptr)

declare ptr @thaw_temporal_calendar_month_code(double, ptr)

declare ptr @thaw_temporal_calendar_era(double, ptr)

declare ptr @thaw_temporal_month_code(double)

declare double @thaw_temporal_duration_from_string(ptr)

declare ptr @thaw_temporal_duration_components_json(ptr)

declare ptr @thaw_temporal_duration_to_string_components(ptr)

declare ptr @thaw_error_name(ptr)

declare ptr @thaw_error_constructor_name(ptr)

declare ptr @thaw_error_message(ptr)

declare ptr @thaw_error_cause(ptr)

declare ptr @thaw_error_code(ptr)

declare ptr @thaw_error_stack(ptr)

declare ptr @thaw_error_stack_with_frames(ptr)

declare ptr @thaw_error_suppressed_error(ptr)

declare ptr @thaw_error_suppressed(ptr)

declare ptr @thaw_error_to_string(ptr)

declare ptr @thaw_error_frame(ptr, ptr)

declare void @thaw_register_fn_table(ptr, i64, ptr, ptr, ptr)

declare double @thaw_error_stack_trace_limit()

declare double @thaw_error_set_stack_trace_limit(double)

declare double @thaw_json_capture_stack_trace(ptr)

declare ptr @thaw_error_frame_own(ptr)

declare i1 @thaw_error_set_prop(ptr, ptr, ptr)

declare i1 @thaw_error_set_prop_value(ptr, ptr, ptr)

declare i1 @thaw_object_stack_set(ptr, ptr)

declare i1 @thaw_object_stack_assign(ptr, ptr)

declare void @thaw_native_subclass_mark(ptr, ptr)

declare ptr @thaw_native_subclass_name(ptr)

declare i1 @thaw_object_stack_visible(ptr)

declare i1 @thaw_object_stack_has(ptr)

declare ptr @thaw_object_stack_get(ptr, ptr)

declare i1 @thaw_error_assign_json(ptr, ptr)

declare i1 @thaw_error_has_prop(ptr, ptr)

declare i1 @thaw_error_delete_prop(ptr, ptr)

declare ptr @thaw_error_get_prop_json(ptr, ptr)

declare ptr @thaw_error_own_keys(ptr, i8)

declare i1 @thaw_error_register_owner_props(ptr, ptr)

declare ptr @thaw_error_owner_props(ptr, i8)

declare ptr @thaw_error_inspect(ptr)

declare ptr @thaw_error_console_inspect(ptr)

declare ptr @thaw_error_entries_json(ptr)

declare ptr @thaw_error_json_stringify(ptr)

declare ptr @thaw_error_property(ptr, ptr)

declare ptr @thaw_error_property_json(ptr, ptr)

declare i1 @thaw_error_is_instance(ptr, ptr)

declare i1 @thaw_error_is_error(ptr)

declare i1 @thaw_native_wrapper_is_live(ptr)

declare i1 @thaw_object_set_state(ptr, i8)

declare i1 @thaw_object_state(ptr, i8)

declare i8 @thaw_object_property_flags(ptr, ptr)

declare i1 @thaw_object_set_property_flags(ptr, ptr, i8)

declare i1 @thaw_object_can_set_property_flags(ptr, ptr, i8)

declare i8 @thaw_arena_contains_allocation(ptr)

declare i1 @thaw_object_register_field_offsets(ptr, ptr, ptr)

declare i64 @thaw_object_field_offset(ptr, ptr, i64)

declare i1 @thaw_object_register_projector(ptr, ptr, ptr)

declare i1 @thaw_object_register_describer(ptr, ptr)

declare ptr @thaw_object_describer(ptr)

declare ptr @thaw_object_projector(ptr, ptr)

declare i1 @thaw_object_copy_state(ptr, ptr)

declare i1 @thaw_object_set_class_identity(ptr, ptr)

declare i1 @thaw_object_copy_class_identity(ptr, ptr)

declare i1 @thaw_object_has_class_identity(ptr, ptr)

declare i1 @thaw_object_hide_marker(ptr, ptr)

declare i1 @thaw_object_reveal_marker(ptr, ptr)

declare i1 @thaw_object_marker_hidden(ptr, ptr)

declare i1 @thaw_object_order_begin(ptr)

declare i1 @thaw_object_order_seed(ptr, ptr)

declare i64 @thaw_object_order_rank(ptr, ptr, i64)

declare i1 @thaw_object_has_full_layout(ptr)

declare i1 @thaw_object_full_has_own(ptr, ptr)

declare ptr @thaw_object_full_own_keys(ptr, i1)

declare i1 @thaw_object_set_accessor(ptr, ptr, ptr, i1)

declare ptr @thaw_object_accessor(ptr, ptr, i1)

declare double @thaw_date_set_full_year(double, double, double, double)

declare double @thaw_date_set_local_full_year(double, double, double, double)

declare double @thaw_date_set_month(double, double, double)

declare double @thaw_date_set_local_month(double, double, double)

declare double @thaw_date_set_seconds(double, double, double)

declare double @thaw_date_set_local_seconds(double, double, double)

declare double @thaw_date_set_hours(double, double, double, double, double)

declare double @thaw_date_set_local_hours(double, double, double, double, double)

declare double @thaw_date_set_minutes(double, double, double, double)

declare double @thaw_date_set_local_minutes(double, double, double, double)

declare double @thaw_date_set_date(double, double)

declare double @thaw_date_set_local_date(double, double)

declare double @thaw_date_set_milliseconds(double, double)

declare double @thaw_date_set_local_milliseconds(double, double)

declare double @thaw_date_local(double, double, double, double, double, double, double)

declare double @thaw_date_utc(double, double, double, double, double, double, double)

declare double @thaw_date_parse(ptr)

declare ptr @thaw_map_new()

declare ptr @thaw_map_snapshot_keys(ptr)

declare ptr @thaw_map_snapshot_values(ptr)

declare ptr @thaw_map_snapshot_entries(ptr)

declare ptr @thaw_set_snapshot_entries(ptr)

declare ptr @thaw_map_iterator_next(ptr, double, double)

declare double @thaw_map_size(ptr)

declare void @thaw_map_clear(ptr)

declare i8 @thaw_map_num_has(ptr, double)

declare i8 @thaw_map_num_set(ptr, double, i64)

declare i8 @thaw_map_num_delete(ptr, double)

declare double @thaw_map_num_get_f64(ptr, double)

declare i8 @thaw_map_num_get_bool(ptr, double)

declare ptr @thaw_map_num_get_ptr(ptr, double)

declare i64 @thaw_map_num_get_i64(ptr, double)

declare i8 @thaw_map_str_has(ptr, ptr)

declare i8 @thaw_map_str_set(ptr, ptr, i64)

declare i8 @thaw_map_str_delete(ptr, ptr)

declare double @thaw_map_str_get_f64(ptr, ptr)

declare i8 @thaw_map_str_get_bool(ptr, ptr)

declare ptr @thaw_map_str_get_ptr(ptr, ptr)

declare i64 @thaw_map_str_get_i64(ptr, ptr)

declare i8 @thaw_map_ref_has(ptr, ptr)

declare i8 @thaw_map_ref_set(ptr, ptr, i64)

declare i8 @thaw_map_ref_delete(ptr, ptr)

declare double @thaw_map_ref_get_f64(ptr, ptr)

declare i8 @thaw_map_ref_get_bool(ptr, ptr)

declare ptr @thaw_map_ref_get_ptr(ptr, ptr)

declare i64 @thaw_map_ref_get_i64(ptr, ptr)

declare i8 @thaw_map_any_has(ptr, ptr)

declare i8 @thaw_map_any_set(ptr, ptr, i64)

declare i8 @thaw_map_any_delete(ptr, ptr)

declare double @thaw_map_any_get_f64(ptr, ptr)

declare i8 @thaw_map_any_get_bool(ptr, ptr)

declare ptr @thaw_map_any_get_ptr(ptr, ptr)

declare i64 @thaw_map_any_get_i64(ptr, ptr)

declare i8 @thaw_map_big_has(ptr, i64)

declare i8 @thaw_map_big_set(ptr, i64, i64)

declare i8 @thaw_map_big_delete(ptr, i64)

declare double @thaw_map_big_get_f64(ptr, i64)

declare i8 @thaw_map_big_get_bool(ptr, i64)

declare ptr @thaw_map_big_get_ptr(ptr, i64)

declare i64 @thaw_map_big_get_i64(ptr, i64)

declare ptr @thaw_string_repeat(ptr, double)

declare ptr @thaw_string_at(ptr, double)

declare ptr @thaw_string_pad_start(ptr, ptr, double)

declare ptr @thaw_string_pad_end(ptr, ptr, double)

declare double @thaw_string_length(ptr)

declare double @thaw_string_char_code_at(ptr, double)

declare double @thaw_string_code_point_at(ptr, double)

declare ptr @thaw_json_as_string(ptr)

declare ptr @thaw_json_as_string_value(ptr)

declare ptr @thaw_json_console_string(ptr)

declare ptr @thaw_json_buffer_inspect(ptr, double, double, double)

declare ptr @thaw_console_format(ptr)

declare ptr @thaw_json_typeof(ptr)

declare ptr @thaw_json_object_tag(ptr)

declare i8 @thaw_json_as_bool(ptr)

declare i8 @thaw_json_is_date_shape(ptr)

declare i8 @thaw_json_is_buffer_shape(ptr)

declare i8 @thaw_json_is_native_iterator(ptr)

declare i8 @thaw_json_is_live_iterable(ptr)

declare i8 @thaw_json_has_wrapper_key(ptr, ptr)

declare i8 @thaw_json_is_error_instance(ptr, ptr)

declare ptr @thaw_json_map_or_set_entries(ptr)

declare ptr @thaw_json_map_or_set_get(ptr, ptr)

declare i8 @thaw_json_map_or_set_has(ptr, ptr)

declare ptr @thaw_json_map_or_set_keys(ptr)

declare ptr @thaw_json_map_or_set_values(ptr)

declare ptr @thaw_json_map_or_set_entries_view(ptr)

declare ptr @thaw_json_map_or_set_set(ptr, ptr, ptr)

declare ptr @thaw_json_map_or_set_add(ptr, ptr)

declare ptr @thaw_json_map_or_set_delete(ptr, ptr)

declare ptr @thaw_json_map_or_set_clear(ptr)

declare double @thaw_json_date_timestamp(ptr)

declare double @thaw_json_date_set_timestamp(ptr, double)

declare ptr @thaw_json_array_new()

declare ptr @thaw_json_receiver_bigint(i64)

declare ptr @thaw_json_receiver_bool(i8)

declare ptr @thaw_json_receiver_number(double)

declare ptr @thaw_json_error_to_string(ptr)

declare ptr @thaw_json_error_stack(ptr)

declare ptr @thaw_json_receiver_string(ptr)

declare ptr @thaw_json_null()

declare ptr @thaw_json_undefined()

declare ptr @thaw_json_function_sentinel(ptr)

declare ptr @thaw_json_accessor_sentinel(i8)

declare void @thaw_function_name_set(ptr, ptr)

declare i8 @thaw_json_is_array(ptr)

declare ptr @thaw_json_array_join(ptr, ptr)

declare ptr @thaw_json_array_hole_mask(ptr)

declare i8 @thaw_json_is_host_arraybuffer(ptr)

declare i8 @thaw_json_is_buffer(ptr)

declare i8 @thaw_json_is_host_error_family(ptr)

declare i8 @thaw_json_is_null(ptr)

declare i8 @thaw_json_is_undefined(ptr)

declare i8 @thaw_json_is_nullish(ptr)

declare i8 @thaw_json_is_primitive_operand(ptr)

declare i8 @thaw_json_is_object_like(ptr)

declare ptr @thaw_json_structured_clone(ptr)

declare ptr @thaw_json_clone(ptr)

declare ptr @thaw_json_share(ptr)

declare ptr @thaw_json_state_key(ptr)

declare ptr @thaw_json_set_prototype(ptr, ptr)

declare i8 @thaw_json_take_prototype_error()

declare ptr @thaw_json_get_prototype(ptr)

declare ptr @thaw_json_keys(ptr)

declare i8 @thaw_json_is_not_iterable(ptr)

declare ptr @thaw_json_for_in_keys(ptr)

declare ptr @thaw_json_own_keys(ptr)

declare ptr @thaw_json_own_symbols(ptr)

declare ptr @thaw_array_keys(ptr, ptr, i8)

declare ptr @thaw_json_values(ptr)

declare ptr @thaw_json_number_values(ptr)

declare ptr @thaw_json_string_values(ptr)

declare ptr @thaw_json_bool_values(ptr)

declare ptr @thaw_json_entries(ptr)

declare ptr @thaw_json_number_entries(ptr)

declare ptr @thaw_json_string_entries(ptr)

declare ptr @thaw_json_bool_entries(ptr)

declare ptr @thaw_json_object_from_number_entries(ptr, ptr)

declare ptr @thaw_json_object_from_string_entries(ptr, ptr)

declare ptr @thaw_json_object_from_bool_entries(ptr, ptr)

declare ptr @thaw_json_object_from_json_entries(ptr, ptr)

declare i8 @thaw_json_take_from_entries_error()

declare i8 @thaw_json_has_own(ptr, ptr)

declare i8 @thaw_json_property_is_enumerable(ptr, ptr)

declare i8 @thaw_json_has(ptr, ptr)

declare ptr @thaw_json_object_assign(ptr, ptr)

declare i8 @thaw_json_take_assign_error()

declare i8 @thaw_json_object_is(ptr, ptr)

declare i8 @thaw_json_object_is_number(ptr, double)

declare i8 @thaw_json_object_is_string(ptr, ptr)

declare i8 @thaw_json_object_is_bool(ptr, i1)

declare void @thaw_json_array_push_hole(ptr)

declare void @thaw_json_array_push_number(ptr, double)

declare void @thaw_json_array_push_string(ptr, ptr)

declare void @thaw_json_array_push_native_wtf8_string(ptr, ptr)

declare void @thaw_json_array_push_bool(ptr, i8)

declare void @thaw_json_array_push_json(ptr, ptr)

declare ptr @thaw_json_from_number_array(ptr)

declare ptr @thaw_json_to_number_array(ptr)

declare i64 @thaw_json_array_length(ptr)

declare ptr @thaw_json_host_collection_snapshot(ptr, i8)

declare void @thaw_json_array_extend(ptr, ptr)

declare ptr @thaw_json_array_assign(ptr, ptr)

declare ptr @thaw_json_buffer_sync(ptr, ptr)

declare ptr @thaw_json_array_slice(ptr, i64)

declare ptr @thaw_json_object_new()

declare ptr @thaw_recursive_lookup(i64)

declare void @thaw_recursive_register(i64, ptr)

declare void @thaw_recursive_unregister(i64)

declare void @thaw_json_object_set_number(ptr, ptr, double)

declare void @thaw_json_object_set_string(ptr, ptr, ptr)

declare void @thaw_json_object_set_bool(ptr, ptr, i8)

declare void @thaw_json_object_set_json(ptr, ptr, ptr)

declare void @thaw_json_object_set_json_owned(ptr, ptr, ptr)

declare i8 @thaw_json_object_delete(ptr, ptr)

declare i8 @thaw_js_load(ptr)

declare i32 @thaw_js_run_event_loop()

declare void @thaw_js_drain_next_ticks()

declare void @thaw_js_collect_garbage()

declare i8 @thaw_js_terminal_work_pending()

declare i32 @thaw_js_run_cli()

declare void @thaw_js_run_until_native_resolved(ptr)

declare ptr @thaw_js_call(ptr, ptr)

declare { ptr, ptr } @thaw_js_call_result(ptr, ptr)

declare i64 @thaw_js_get_global(ptr)

declare ptr @thaw_js_handle_to_string(i64)

declare extern_weak ptr @thaw_js_handle_typed_array_probe(i64)

declare extern_weak i8 @thaw_js_handle_is_plain_data(i64)

declare ptr @thaw_console_typed_or(ptr, ptr)

declare extern_weak ptr @thaw_js_handle_to_console_string(i64)

declare { ptr, ptr } @thaw_js_call_handle_result(i64, ptr)

declare { i64, ptr } @thaw_js_call_handle_handle_result(i64, ptr, i1)

declare { i64, ptr } @thaw_js_new_function(ptr)

declare { ptr, ptr } @thaw_js_call_handle_value_result(i64, i64)

declare i8 @thaw_js_retain_handle(i64)

declare i8 @thaw_js_release_handle(i64)

declare i64 @thaw_js_release_all_handles()

declare i8 @thaw_js_shutdown_context()

declare { i64, ptr } @thaw_js_intern_native_object(ptr, ptr, i64)

declare { i64, ptr } @thaw_js_register_native_method_callback_graph(ptr, ptr, i64, i64, i8, ptr, i8)

declare { i64, ptr } @thaw_js_lookup_native_object(ptr, ptr)

declare { i64, ptr } @thaw_js_error_from_tagged_handle(ptr)

declare i8 @thaw_js_engine_id()

declare void @thaw_json_link_native_object_by_serial(i64, ptr, ptr)

declare void @thaw_json_link_native_tree_by_serial(i64, ptr, ptr)

declare void @thaw_json_note_view_copy()

declare i8 @thaw_json_has_native_link(ptr)

declare ptr @thaw_json_linked_array(ptr)

declare i64 @thaw_js_origin_serial(i64)

declare void @thaw_json_link_native_by_serial(i64, ptr, i8)

declare extern_weak { i64, ptr } @thaw_js_native_object_pointer(i64, ptr)

declare { ptr, ptr } @thaw_js_call_selected_method_graph_result(i64, i64, ptr, ptr)

declare { i64, ptr } @thaw_js_call_selected_method_handle_graph_args_result(i64, i64, ptr, ptr, i1)

declare { i64, ptr } @thaw_js_get_property_result(i64, ptr)

declare { i64, ptr } @thaw_js_get_property_json_key_result(i64, ptr)

declare { i64, ptr } @thaw_js_delete_property_result(i64, ptr)

declare { i64, ptr } @thaw_js_has_property_result(i64, ptr)

declare { ptr, ptr } @thaw_js_host_query_result(i64, i8)

declare { ptr, ptr } @thaw_js_host_date_set_result(i64, double)

declare { i64, ptr } @thaw_js_set_property_graph_result(i64, ptr, ptr)

declare { i64, ptr } @thaw_js_property_predicate_json_key_result(i64, ptr, i8)

declare { ptr, ptr } @thaw_js_host_enumerate_result(i64, i8)

declare { i64, ptr } @thaw_js_retain_json_result(ptr)

declare { i64, ptr } @thaw_js_event_key_handle_result(ptr, i8)

declare { i64, ptr } @thaw_js_retain_graph_result(ptr)

declare { i64, ptr } @thaw_js_set_property_result(i64, ptr, i64)

declare { ptr, ptr } @thaw_js_call_method_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_js_set_property_json_result(i64, ptr, ptr)

declare { i64, ptr } @thaw_js_call_method_handle_result(i64, ptr, ptr, i1)

declare { ptr, ptr } @thaw_js_resolve_handle_result(i64)

declare { i64, ptr } @thaw_js_resolve_handle_handle_result(i64)

declare { ptr, ptr } @thaw_js_call_handle_mixed_result(i64, ptr, ptr)

declare { i64, i64, i8, ptr } @thaw_js_get_error_constructor_exact_result(ptr)

declare { i64, i64, i8, ptr } @thaw_js_invoke_handle_graph_exact_result(i64, ptr, i8)

declare { i64, i64, i8, ptr } @thaw_js_get_property_exact_result(i64, ptr)

declare { i64, i64, i8, ptr } @thaw_js_iterator_step_exact_result(i64, i8, ptr)

declare { i64, i64, i8, ptr } @thaw_js_to_iterator_exact_result(i64)

declare { i64, i64, i8, ptr } @thaw_js_to_async_iterator_exact_result(i64)

declare { i64, i64, i8, ptr } @thaw_js_call_handle_mixed_exact_consuming_result(i64, ptr, ptr)

declare { i64, i64, i8, ptr } @thaw_js_call_handle_mixed_exact_resolved_consuming_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_js_call_handle_mixed_native_json_result(i64, ptr, ptr, ptr)

declare { i64, ptr } @thaw_js_call_handle_mixed_handle_result(i64, ptr, ptr)

declare void @thaw_js_release_native_handle_array(ptr)

declare { i64, ptr } @thaw_js_construct_handle_result(i64, ptr)

declare { i64, ptr } @thaw_js_call_handle_handle_graph_args_result(i64, ptr, i1)

declare { i64, ptr } @thaw_js_call_method_handle_graph_args_result(i64, ptr, ptr, i1)

declare { i64, ptr } @thaw_js_call_handle_mixed_handle_graph_args_result(i64, ptr, ptr)

declare { i64, ptr } @thaw_js_call_handle_mixed_handle_graph_args_consuming_result(i64, ptr, ptr)

declare { i64, ptr } @thaw_js_construct_handle_graph_args_result(i64, ptr)

declare { ptr, ptr } @thaw_js_call_graph_result(ptr, ptr)

declare { ptr, ptr } @thaw_js_set_property_json_graph_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_js_call_handle_graph_result(i64, ptr)

declare { ptr, ptr } @thaw_js_call_handle_with_this_graph_wire_result(i64, ptr)

declare { ptr, ptr } @thaw_js_call_handle_value_graph_result(i64, i64)

declare { ptr, ptr } @thaw_js_call_method_graph_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_js_resolve_handle_graph_result(i64)

declare { ptr, ptr } @thaw_js_capture_handle_graph_result(i64)

declare { ptr, ptr } @thaw_js_call_handle_mixed_graph_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_js_call_handle_mixed_native_json_graph_result(i64, ptr, ptr, ptr)

declare { i64, ptr } @thaw_js_register_native_callback_graph(ptr, ptr, i64, i64, i8, ptr, i8)

declare { i64, ptr } @thaw_js_register_native_callback(ptr, ptr, i64, i64, i8, ptr, i8)

declare { double, ptr } @thaw_jit_call_f64(ptr, ptr, i64, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr)

declare double @thaw_js_dynamic_object_query(i8, i64, ptr)

declare i8 @thaw_napi_load(ptr)

declare i8 @thaw_napi_load_named(ptr, ptr)

declare i8 @thaw_napi_load_named_qualified(ptr, ptr, ptr)

declare i8 @thaw_napi_load_embedded_hex(ptr, ptr)

declare i8 @thaw_napi_load_embedded_hex_qualified(ptr, ptr, ptr)

declare i8 @thaw_napi_load_embedded_shared_hex(ptr)

declare ptr @thaw_napi_embed_executable_hex(ptr)

declare i8 @thaw_js_set_process_env(ptr, ptr)

declare i8 @thaw_napi_load_shared(ptr)

declare { ptr, ptr } @thaw_napi_call_result(ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_typed_result(ptr, ptr)

declare { ptr, ptr } @thaw_tjs_call_typed_result(ptr, ptr)

declare i64 @thaw_tjs_take_thrown_handle()

declare ptr @thaw_json_adopt_handle_marker(ptr)

declare ptr @thaw_tjs_frame_error(ptr)

declare void @thaw_tjs_watch_promise(i64, ptr, ptr)

declare { ptr, ptr } @thaw_tjs_class_call(ptr, ptr, i64, i8)

declare void @thaw_tjs_callback_identity(ptr)

declare i64 @thaw_tjs_register_callback(ptr, ptr, i64)

declare { i64, ptr } @thaw_tjs_retain_graph_result(ptr)

declare i8 @thaw_tjs_load_script(ptr)

declare i8 @thaw_tjs_register(ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_handle_typed_result(i64, ptr)

declare { i64, ptr } @thaw_napi_call_export_handle_typed_result(ptr, ptr)

declare { i64, ptr } @thaw_napi_call_export_handle_with_function_typed_result(ptr, ptr, i64, ptr, ptr)

declare { i64, ptr } @thaw_napi_call_export_handle_with_functions_typed_result(ptr, ptr, ptr, i64)

declare { ptr, ptr } @thaw_napi_call_with_functions_typed_result(ptr, ptr, ptr, i64)

declare i64 @thaw_napi_get_export(ptr)

declare { i64, ptr } @thaw_napi_get_export_typed_result(ptr)

declare { i64, ptr } @thaw_napi_construct_handle_result(i64, ptr)

declare { i64, ptr } @thaw_napi_construct_handle_typed_result(i64, ptr)

declare { ptr, ptr } @thaw_napi_call_method_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_method_typed_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_napi_get_property_result(i64, ptr)

declare { ptr, ptr } @thaw_napi_get_property_typed_result(i64, ptr)

declare { ptr, ptr } @thaw_napi_set_property_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_napi_set_property_typed_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_method_with_callback_result(i64, ptr, ptr, ptr, ptr, i8)

declare { ptr, ptr } @thaw_napi_call_with_callback_result(ptr, ptr, ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_graph_result(ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_typed_graph_result(ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_handle_typed_graph_result(i64, ptr)

declare { ptr, ptr } @thaw_napi_call_with_functions_typed_graph_result(ptr, ptr, ptr, i64)

declare { ptr, ptr } @thaw_napi_call_method_typed_graph_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_napi_get_property_typed_graph_result(i64, ptr)

declare { ptr, ptr } @thaw_napi_set_property_typed_graph_result(i64, ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_with_callback_graph_result(ptr, ptr, ptr, ptr)

declare { ptr, ptr } @thaw_napi_call_method_with_callback_graph_result(i64, ptr, ptr, ptr, ptr, i8)

declare { i64, ptr } @thaw_napi_call_export_handle_typed_graph_args_result(ptr, ptr)

declare { i64, ptr } @thaw_napi_call_export_handle_with_functions_typed_graph_args_result(ptr, ptr, ptr, i64)

declare { i64, ptr } @thaw_napi_construct_handle_typed_graph_args_result(i64, ptr)

declare i64 @thaw_napi_run_async_work()

declare i64 @thaw_napi_poll_async_work()

declare i8 @thaw_napi_async_work_pending()

declare i8 @thaw_napi_begin_shutdown()

declare i8 @thaw_napi_poll_shutdown()

declare i8 @thaw_napi_finish_shutdown()

declare ptr @thaw_napi_take_shutdown_error()

declare i8 @thaw_napi_unload_all()

declare i8 @thaw_napi_take_fatal_exception()

declare i64 @thaw_timer_set(i8, double, ptr, ptr)

declare void @thaw_timer_clear(i64)

declare void @thaw_timer_ref(i64)

declare void @thaw_timer_unref(i64)

declare void @thaw_timer_refresh(i64)

declare i8 @thaw_timer_has_ref(i64)

declare ptr @thaw_sleep_ms(i64)

declare ptr @thaw_http_get_async(ptr)

declare ptr @thaw_runtime_run_until_resolved(ptr)

declare ptr @thaw_runtime_await_blocking(ptr)

declare void @thaw_runtime_abort_pending_work()

declare void @thaw_js_fatal_exit()

declare void @thaw_runtime_report_unhandled()

declare i64 @thaw_runtime_drain_detached()

declare i64 @thaw_runtime_run_until_idle()

declare i8 @thaw_runtime_async_work_pending()

declare void @thaw_runtime_shutdown_wait()

declare ptr @thaw_promise_new()

declare i8 @thaw_promise_retain(ptr)

declare i8 @thaw_promise_arena_slot_replace(ptr, ptr, ptr)

declare i8 @thaw_promise_subscribe(ptr, ptr, ptr)

declare void @thaw_promise_await_outer(ptr)

declare i8 @thaw_promise_subscribe_await(ptr, ptr, ptr)

declare i8 @thaw_promise_resolve(ptr, ptr)

declare i8 @thaw_promise_reject(ptr, ptr)

declare i8 @thaw_promise_reject_native_text(ptr, ptr)

declare i8 @thaw_promise_reject_text_copy(ptr, ptr)

declare i8 @thaw_promise_forward_rejection(ptr, ptr, ptr)

declare i8 @thaw_promise_reject_typed(ptr, ptr, i64, double, i64, i1, ptr)

declare i8 @thaw_promise_reject_typed_with_native_text(ptr, ptr, i64, double, i64, i1, ptr, ptr)

declare i8 @thaw_promise_reject_typed_with_aggregate(ptr, ptr, i64, double, i64, i1, ptr, ptr, ptr)

declare i8 @thaw_promise_reject_typed_with_native_provenance(ptr, ptr, i64, double, i64, i1, ptr, ptr, ptr, ptr)

declare ptr @thaw_promise_exception_aggregate_errors(ptr)

declare i64 @thaw_promise_exception_tag(ptr)

declare double @thaw_promise_exception_f64(ptr)

declare i64 @thaw_promise_exception_i64(ptr)

declare i1 @thaw_promise_exception_bool(ptr)

declare ptr @thaw_promise_exception_object(ptr)

declare i8 @thaw_promise_state(ptr)

declare ptr @thaw_promise_result_ptr(ptr)

declare i8 @thaw_runtime_poll_one()

declare void @thaw_promise_destroy(ptr)

declare i8 @thaw_promise_detach(ptr, ptr)

declare i8 @thaw_promise_detach_unhandled(ptr)

declare i8 @thaw_promise_detach_for_report(ptr, ptr)

declare i8 @thaw_promise_drain_unhandled(ptr)

declare void @thaw_promise_set_unhandled_reporter(ptr)

declare void @thaw_promise_set_rejection_handled_reporter(ptr)

declare i8 @thaw_promise_drain_unhandled_result(ptr)

declare i8 @thaw_promise_drain_unhandled_text_result(ptr)

declare void @thaw_promise_set_unhandled_reporter_result(ptr)

declare void @thaw_promise_set_unhandled_reporter_text_result(ptr)

declare void @thaw_promise_set_rejection_handled_reporter_result(ptr)

declare i8 @thaw_promise_take_unhandled_failure()

declare i64 @thaw_promise_take_report_activity()

declare ptr @thaw_promise_chain(ptr, ptr, ptr, i8)

declare ptr @thaw_promise_chain_both(ptr, ptr, ptr, ptr, ptr)

declare i8 @thaw_promise_adopt(ptr, ptr)

declare ptr @thaw_promise_finally(ptr, ptr, ptr)

declare i8 @thaw_promise_finally_adopt(ptr, ptr, ptr, i8, i64, double, i64, i1, ptr)

declare ptr @thaw_promise_exception_native_text_copy(ptr)

declare ptr @thaw_promise_exception_native(ptr)

declare i8 @thaw_promise_finally_adopt_with_source(ptr, ptr, ptr, i8, i64, double, i64, i1, ptr, ptr)

declare ptr @thaw_promise_all_slots(ptr, i64, i64)

declare ptr @thaw_promise_all_typed(ptr, ptr, i64)

declare ptr @thaw_promise_race(ptr, i64)

declare ptr @thaw_promise_any(ptr, i64)

declare ptr @thaw_promise_all_settled(ptr, i64, i64)

define internal ptr @f() #1 section "thaw_user_fns" {
entry:
  %async_frame = call ptr @thaw_arena_alloc(i64 40, i64 8)
  %async_frame_is_null = icmp eq ptr %async_frame, null
  br i1 %async_frame_is_null, label %async_frame_allocation_failed, label %async_frame_allocation_ready

async_frame_allocation_failed:                    ; preds = %entry
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message)
  ret ptr null

async_frame_allocation_ready:                     ; preds = %entry
  %async_completion = call ptr @thaw_promise_new()
  %async_completion_is_null = icmp eq ptr %async_completion, null
  br i1 %async_completion_is_null, label %async_completion_allocation_failed, label %async_completion_allocation_ready

async_completion_allocation_failed:               ; preds = %async_frame_allocation_ready
  %synthetic_error_frame1 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.1)
  ret ptr null

async_completion_allocation_ready:                ; preds = %async_frame_allocation_ready
  %completion_slot = getelementptr inbounds i8, ptr %async_frame, i64 0
  store ptr null, ptr %completion_slot, align 8
  %replace_arena_promise_slot = call i8 @thaw_promise_arena_slot_replace(ptr %async_frame, ptr %completion_slot, ptr %async_completion)
  %arena_promise_slot_replace_failed = icmp eq i8 %replace_arena_promise_slot, 0
  br i1 %arena_promise_slot_replace_failed, label %arena_promise_slot_error, label %arena_promise_slot_ready

arena_promise_slot_error:                         ; preds = %async_completion_allocation_ready
  %synthetic_error_frame2 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.2)
  ret ptr null

arena_promise_slot_ready:                         ; preds = %async_completion_allocation_ready
  %waiting_slot = getelementptr inbounds i8, ptr %async_frame, i64 16
  store ptr null, ptr %waiting_slot, align 8
  %frame___thaw_thrown_value_0 = getelementptr inbounds i8, ptr %async_frame, i64 24
  %frame___thaw_thrown_value_03 = getelementptr inbounds i8, ptr %async_frame, i64 24
  %error_frame = call ptr @thaw_error_frame(ptr @strlit, ptr @strlit.3)
  store ptr %error_frame, ptr %frame___thaw_thrown_value_03, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 4, ptr @__thaw_pending_exception_value_tag, align 4
  %__thaw_thrown_value_0 = load ptr, ptr %frame___thaw_thrown_value_03, align 8
  store ptr %__thaw_thrown_value_0, ptr @__thaw_pending_exception_native_text, align 8
  %promise_exception_value = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %promise_exception_value4 = load double, ptr @__thaw_pending_exception_f64, align 8
  %promise_exception_value5 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %promise_exception_value6 = load i1, ptr @__thaw_pending_exception_bool, align 1
  %promise_exception_value7 = load ptr, ptr @__thaw_pending_exception_object, align 8
  %pending_native_text_provenance = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %pending_aggregate_errors = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %pending_native_exception_descriptor = load ptr, ptr @__thaw_pending_exception_native, align 8
  %pending_error_bits = ptrtoint ptr %__thaw_thrown_value_0 to i64
  %pending_native_text_bits = ptrtoint ptr %pending_native_text_provenance to i64
  %aggregate_text_matches_error = icmp eq i64 %pending_error_bits, %pending_native_text_bits
  %aggregate_has_trusted_text = icmp ne ptr %pending_native_text_provenance, null
  %aggregate_trusted_pair = and i1 %aggregate_text_matches_error, %aggregate_has_trusted_text
  %paired_aggregate_errors = select i1 %aggregate_trusted_pair, ptr %pending_aggregate_errors, ptr null
  %reject_completion = call i8 @thaw_promise_reject_typed_with_native_provenance(ptr %async_completion, ptr %__thaw_thrown_value_0, i64 %promise_exception_value, double %promise_exception_value4, i64 %promise_exception_value5, i1 %promise_exception_value6, ptr %promise_exception_value7, ptr %pending_native_text_provenance, ptr %paired_aggregate_errors, ptr %pending_native_exception_descriptor)
  ret ptr %async_completion
}

; Function Attrs: noinline
define internal ptr @thaw_user_main() #2 section "thaw_user_fns" {
entry:
  %async_frame = call ptr @thaw_arena_alloc(i64 24, i64 8)
  %async_frame_is_null = icmp eq ptr %async_frame, null
  br i1 %async_frame_is_null, label %async_frame_allocation_failed, label %async_frame_allocation_ready

async_frame_allocation_failed:                    ; preds = %entry
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message)
  ret ptr null

async_frame_allocation_ready:                     ; preds = %entry
  %async_completion = call ptr @thaw_promise_new()
  %async_completion_is_null = icmp eq ptr %async_completion, null
  br i1 %async_completion_is_null, label %async_completion_allocation_failed, label %async_completion_allocation_ready

async_completion_allocation_failed:               ; preds = %async_frame_allocation_ready
  %synthetic_error_frame1 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.1)
  ret ptr null

async_completion_allocation_ready:                ; preds = %async_frame_allocation_ready
  %completion_slot = getelementptr inbounds i8, ptr %async_frame, i64 0
  store ptr null, ptr %completion_slot, align 8
  %replace_arena_promise_slot = call i8 @thaw_promise_arena_slot_replace(ptr %async_frame, ptr %completion_slot, ptr %async_completion)
  %arena_promise_slot_replace_failed = icmp eq i8 %replace_arena_promise_slot, 0
  br i1 %arena_promise_slot_replace_failed, label %arena_promise_slot_error, label %arena_promise_slot_ready

arena_promise_slot_error:                         ; preds = %async_completion_allocation_ready
  %synthetic_error_frame2 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.2)
  ret ptr null

arena_promise_slot_ready:                         ; preds = %async_completion_allocation_ready
  %waiting_slot = getelementptr inbounds i8, ptr %async_frame, i64 16
  store ptr null, ptr %waiting_slot, align 8
  %calltmp = call ptr @f()
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

call_ok:                                          ; preds = %arena_promise_slot_ready
  %detach_promise = call i8 @thaw_promise_detach_unhandled(ptr %calltmp)
  %closure_alloc = call ptr @thaw_arena_alloc(i64 16, i64 8)
  store ptr @__thaw_lambda_0, ptr %closure_alloc, align 8
  %closure_this_entry = getelementptr inbounds i8, ptr %closure_alloc, i64 8
  store ptr @__thaw_lambda_0__thaw_this_adapter, ptr %closure_this_entry, align 8
  %callable_is_undefined = icmp eq ptr %closure_alloc, null
  br i1 %callable_is_undefined, label %undefined_callable, label %present_callable

propagate_exception:                              ; preds = %arena_promise_slot_ready
  %promise_exception_value = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %promise_exception_value3 = load double, ptr @__thaw_pending_exception_f64, align 8
  %promise_exception_value4 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %promise_exception_value5 = load i1, ptr @__thaw_pending_exception_bool, align 1
  %promise_exception_value6 = load ptr, ptr @__thaw_pending_exception_object, align 8
  %pending_native_text_provenance = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %pending_aggregate_errors = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %pending_native_exception_descriptor = load ptr, ptr @__thaw_pending_exception_native, align 8
  %pending_error_bits = ptrtoint ptr %pending_exception to i64
  %pending_native_text_bits = ptrtoint ptr %pending_native_text_provenance to i64
  %aggregate_text_matches_error = icmp eq i64 %pending_error_bits, %pending_native_text_bits
  %aggregate_has_trusted_text = icmp ne ptr %pending_native_text_provenance, null
  %aggregate_trusted_pair = and i1 %aggregate_text_matches_error, %aggregate_has_trusted_text
  %paired_aggregate_errors = select i1 %aggregate_trusted_pair, ptr %pending_aggregate_errors, ptr null
  %reject_frame_exception = call i8 @thaw_promise_reject_typed_with_native_provenance(ptr %async_completion, ptr %pending_exception, i64 %promise_exception_value, double %promise_exception_value3, i64 %promise_exception_value4, i1 %promise_exception_value5, ptr %promise_exception_value6, ptr %pending_native_text_provenance, ptr %paired_aggregate_errors, ptr %pending_native_exception_descriptor)
  store ptr null, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  ret ptr %async_completion

await_operand_rejected:                           ; preds = %promise_reject_closure_failed, %promise_resolve_closure_failed, %resolver_state_failed, %promise_new_failed, %undefined_callable
  %await_operand_error = load ptr, ptr @__thaw_pending_exception, align 8
  %await_rejected_promise = call ptr @thaw_promise_new()
  %promise_exception_value33 = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %promise_exception_value34 = load double, ptr @__thaw_pending_exception_f64, align 8
  %promise_exception_value35 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %promise_exception_value36 = load i1, ptr @__thaw_pending_exception_bool, align 1
  %promise_exception_value37 = load ptr, ptr @__thaw_pending_exception_object, align 8
  %pending_native_text_provenance38 = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %pending_aggregate_errors39 = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %pending_native_exception_descriptor40 = load ptr, ptr @__thaw_pending_exception_native, align 8
  %pending_error_bits41 = ptrtoint ptr %await_operand_error to i64
  %pending_native_text_bits42 = ptrtoint ptr %pending_native_text_provenance38 to i64
  %aggregate_text_matches_error43 = icmp eq i64 %pending_error_bits41, %pending_native_text_bits42
  %aggregate_has_trusted_text44 = icmp ne ptr %pending_native_text_provenance38, null
  %aggregate_trusted_pair45 = and i1 %aggregate_text_matches_error43, %aggregate_has_trusted_text44
  %paired_aggregate_errors46 = select i1 %aggregate_trusted_pair45, ptr %pending_aggregate_errors39, ptr null
  %reject_await_operand = call i8 @thaw_promise_reject_typed_with_native_provenance(ptr %await_rejected_promise, ptr %await_operand_error, i64 %promise_exception_value33, double %promise_exception_value34, i64 %promise_exception_value35, i1 %promise_exception_value36, ptr %promise_exception_value37, ptr %pending_native_text_provenance38, ptr %paired_aggregate_errors46, ptr %pending_native_exception_descriptor40)
  store ptr null, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  br label %await_operand_ready

undefined_callable:                               ; preds = %call_ok
  %synthetic_error_frame7 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.11)
  br label %await_operand_rejected

present_callable:                                 ; preds = %call_ok
  %promise_new = call ptr @thaw_promise_new()
  %promise_new_missing = icmp eq ptr %promise_new, null
  br i1 %promise_new_missing, label %promise_new_failed, label %promise_new_ready

promise_new_failed:                               ; preds = %present_callable
  %synthetic_error_frame8 = call ptr @thaw_set_synthetic_error(ptr @strlit, ptr @synthetic_error_message.12)
  br label %await_operand_rejected

promise_new_ready:                                ; preds = %present_callable
  %resolver_state = call ptr @thaw_arena_alloc(i64 16, i64 8)
  %resolver_state_missing = icmp eq ptr %resolver_state, null
  br i1 %resolver_state_missing, label %resolver_state_failed, label %resolver_state_ready

resolver_state_failed:                            ; preds = %promise_new_ready
  call void @thaw_promise_destroy(ptr %promise_new)
  %synthetic_error_frame9 = call ptr @thaw_set_synthetic_error(ptr @strlit, ptr @synthetic_error_message.13)
  br label %await_operand_rejected

resolver_state_ready:                             ; preds = %promise_new_ready
  store ptr %promise_new, ptr %resolver_state, align 8
  %resolver_called_slot = getelementptr inbounds i8, ptr %resolver_state, i64 8
  store i8 0, ptr %resolver_called_slot, align 1
  %resolve_closure = call ptr @thaw_arena_alloc(i64 24, i64 8)
  %special_closure_allocation_failed = icmp eq ptr %resolve_closure, null
  br i1 %special_closure_allocation_failed, label %special_closure_missing, label %special_closure_ready

special_closure_missing:                          ; preds = %resolver_state_ready
  br label %special_closure_done

special_closure_ready:                            ; preds = %resolver_state_ready
  store ptr @__thaw_promise_resolve_11, ptr %resolve_closure, align 8
  %special_closure_this_entry = getelementptr inbounds i8, ptr %resolve_closure, i64 8
  store ptr @resolve_closure__thaw_this_adapter, ptr %special_closure_this_entry, align 8
  %promise_capture = getelementptr inbounds i8, ptr %resolve_closure, i64 16
  store ptr %resolver_state, ptr %promise_capture, align 8
  br label %special_closure_done

special_closure_done:                             ; preds = %special_closure_ready, %special_closure_missing
  %special_closure_or_null = phi ptr [ null, %special_closure_missing ], [ %resolve_closure, %special_closure_ready ]
  %missing_resolve_closure = icmp eq ptr %special_closure_or_null, null
  br i1 %missing_resolve_closure, label %promise_resolve_closure_failed, label %promise_resolve_closure_ready

promise_resolve_closure_failed:                   ; preds = %special_closure_done
  call void @thaw_promise_destroy(ptr %promise_new)
  %synthetic_error_frame10 = call ptr @thaw_set_synthetic_error(ptr @strlit, ptr @synthetic_error_message.14)
  br label %await_operand_rejected

promise_resolve_closure_ready:                    ; preds = %special_closure_done
  %reject_closure = call ptr @thaw_arena_alloc(i64 24, i64 8)
  %special_closure_allocation_failed14 = icmp eq ptr %reject_closure, null
  br i1 %special_closure_allocation_failed14, label %special_closure_missing11, label %special_closure_ready12

special_closure_missing11:                        ; preds = %promise_resolve_closure_ready
  br label %special_closure_done13

special_closure_ready12:                          ; preds = %promise_resolve_closure_ready
  store ptr @__thaw_promise_reject_12, ptr %reject_closure, align 8
  %special_closure_this_entry15 = getelementptr inbounds i8, ptr %reject_closure, i64 8
  store ptr @reject_closure__thaw_this_adapter, ptr %special_closure_this_entry15, align 8
  %promise_capture16 = getelementptr inbounds i8, ptr %reject_closure, i64 16
  store ptr %resolver_state, ptr %promise_capture16, align 8
  br label %special_closure_done13

special_closure_done13:                           ; preds = %special_closure_ready12, %special_closure_missing11
  %special_closure_or_null17 = phi ptr [ null, %special_closure_missing11 ], [ %reject_closure, %special_closure_ready12 ]
  %missing_reject_closure = icmp eq ptr %special_closure_or_null17, null
  br i1 %missing_reject_closure, label %promise_reject_closure_failed, label %promise_reject_closure_ready

promise_reject_closure_failed:                    ; preds = %special_closure_done13
  call void @thaw_promise_destroy(ptr %promise_new)
  %synthetic_error_frame18 = call ptr @thaw_set_synthetic_error(ptr @strlit, ptr @synthetic_error_message.15)
  br label %await_operand_rejected

promise_reject_closure_ready:                     ; preds = %special_closure_done13
  %executor_code = load ptr, ptr %closure_alloc, align 8
  call void %executor_code(ptr %closure_alloc, ptr %special_closure_or_null, ptr %special_closure_or_null17)
  %executor_error = load ptr, ptr @__thaw_pending_exception, align 8
  %executor_has_error = icmp ne ptr %executor_error, null
  br i1 %executor_has_error, label %executor_rejected, label %executor_complete

executor_rejected:                                ; preds = %promise_reject_closure_ready
  %executor_resolver_called = load i8, ptr %resolver_called_slot, align 1
  %executor_resolver_uncalled = icmp eq i8 %executor_resolver_called, 0
  br i1 %executor_resolver_uncalled, label %executor_reject_first, label %executor_clear_error

executor_complete:                                ; preds = %executor_clear_error, %promise_reject_closure_ready
  br label %await_operand_ready

executor_reject_first:                            ; preds = %executor_rejected
  store i8 1, ptr %resolver_called_slot, align 1
  %promise_exception_value19 = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %promise_exception_value20 = load double, ptr @__thaw_pending_exception_f64, align 8
  %promise_exception_value21 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %promise_exception_value22 = load i1, ptr @__thaw_pending_exception_bool, align 1
  %promise_exception_value23 = load ptr, ptr @__thaw_pending_exception_object, align 8
  %pending_native_text_provenance24 = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %pending_aggregate_errors25 = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %pending_native_exception_descriptor26 = load ptr, ptr @__thaw_pending_exception_native, align 8
  %pending_error_bits27 = ptrtoint ptr %executor_error to i64
  %pending_native_text_bits28 = ptrtoint ptr %pending_native_text_provenance24 to i64
  %aggregate_text_matches_error29 = icmp eq i64 %pending_error_bits27, %pending_native_text_bits28
  %aggregate_has_trusted_text30 = icmp ne ptr %pending_native_text_provenance24, null
  %aggregate_trusted_pair31 = and i1 %aggregate_text_matches_error29, %aggregate_has_trusted_text30
  %paired_aggregate_errors32 = select i1 %aggregate_trusted_pair31, ptr %pending_aggregate_errors25, ptr null
  %reject_executor_throw = call i8 @thaw_promise_reject_typed_with_native_provenance(ptr %promise_new, ptr %executor_error, i64 %promise_exception_value19, double %promise_exception_value20, i64 %promise_exception_value21, i1 %promise_exception_value22, ptr %promise_exception_value23, ptr %pending_native_text_provenance24, ptr %paired_aggregate_errors32, ptr %pending_native_exception_descriptor26)
  br label %executor_clear_error

executor_clear_error:                             ; preds = %executor_reject_first, %executor_rejected
  store ptr null, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  br label %executor_complete

await_operand_ready:                              ; preds = %await_operand_rejected, %executor_complete
  %await_operand = phi ptr [ %promise_new, %executor_complete ], [ %await_rejected_promise, %await_operand_rejected ]
  %waiting_slot47 = getelementptr inbounds i8, ptr %async_frame, i64 16
  store ptr %await_operand, ptr %waiting_slot47, align 8
  %state_slot = getelementptr inbounds i8, ptr %async_frame, i64 8
  store i64 1, ptr %state_slot, align 4
  %subscribe_resume = call i8 @thaw_promise_subscribe_await(ptr %await_operand, ptr @thaw_user_main.resume, ptr %async_frame)
  ret ptr %async_completion
}

define internal void @f.resume(ptr %0, ptr %1) #1 section "thaw_user_fns" {
entry:
  %frame___thaw_thrown_value_0 = getelementptr inbounds i8, ptr %0, i64 24
  %waiting_slot = getelementptr inbounds i8, ptr %0, i64 16
  %waiting_promise = load ptr, ptr %waiting_slot, align 8
  %completion_slot = getelementptr inbounds i8, ptr %0, i64 0
  %completion = load ptr, ptr %completion_slot, align 8
  %state_slot = getelementptr inbounds i8, ptr %0, i64 8
  %async_state = load i64, ptr %state_slot, align 4
  %synchronous_transition = icmp eq ptr %waiting_promise, null
  br i1 %synchronous_transition, label %resume_fulfilled, label %inspect_waiting

resume_fulfilled:                                 ; preds = %inspect_waiting, %entry
  call void @thaw_promise_destroy(ptr %waiting_promise)
  store ptr null, ptr %waiting_slot, align 8
  switch i64 %async_state, label %invalid_state [
  ]

inspect_waiting:                                  ; preds = %entry
  %waiting_state = call i8 @thaw_promise_state(ptr %waiting_promise)
  %waiting_rejected = icmp eq i8 %waiting_state, 2
  br i1 %waiting_rejected, label %handle_rejection, label %resume_fulfilled

handle_rejection:                                 ; preds = %inspect_waiting
  switch i64 %async_state, label %propagate_rejection [
  ]

propagate_rejection:                              ; preds = %handle_rejection
  %reject_completion = call i8 @thaw_promise_forward_rejection(ptr %completion, ptr %waiting_promise, ptr %1)
  call void @thaw_promise_destroy(ptr %waiting_promise)
  store ptr null, ptr %waiting_slot, align 8
  ret void

invalid_state:                                    ; preds = %resume_fulfilled
  ret void
}

define internal ptr @thaw_set_synthetic_error(ptr %0, ptr %1) #1 {
entry:
  %synthetic_error_frame = call ptr @thaw_error_frame(ptr %0, ptr %1)
  store ptr %synthetic_error_frame, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %synthetic_error_frame, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  ret ptr %synthetic_error_frame
}

define internal void @thaw_user_main.resume(ptr %0, ptr %1) #1 section "thaw_user_fns" {
entry:
  %waiting_slot = getelementptr inbounds i8, ptr %0, i64 16
  %waiting_promise = load ptr, ptr %waiting_slot, align 8
  %completion_slot = getelementptr inbounds i8, ptr %0, i64 0
  %completion = load ptr, ptr %completion_slot, align 8
  %state_slot = getelementptr inbounds i8, ptr %0, i64 8
  %async_state = load i64, ptr %state_slot, align 4
  %synchronous_transition = icmp eq ptr %waiting_promise, null
  br i1 %synchronous_transition, label %resume_fulfilled, label %inspect_waiting

resume_fulfilled:                                 ; preds = %inspect_waiting, %entry
  call void @thaw_promise_destroy(ptr %waiting_promise)
  store ptr null, ptr %waiting_slot, align 8
  switch i64 %async_state, label %invalid_state [
    i64 1, label %state_1
  ]

inspect_waiting:                                  ; preds = %entry
  %waiting_state = call i8 @thaw_promise_state(ptr %waiting_promise)
  %waiting_rejected = icmp eq i8 %waiting_state, 2
  br i1 %waiting_rejected, label %handle_rejection, label %resume_fulfilled

handle_rejection:                                 ; preds = %inspect_waiting
  switch i64 %async_state, label %propagate_rejection [
  ]

propagate_rejection:                              ; preds = %handle_rejection
  %reject_completion = call i8 @thaw_promise_forward_rejection(ptr %completion, ptr %waiting_promise, ptr %1)
  call void @thaw_promise_destroy(ptr %waiting_promise)
  store ptr null, ptr %waiting_slot, align 8
  ret void

invalid_state:                                    ; preds = %resume_fulfilled
  ret void

state_1:                                          ; preds = %resume_fulfilled
  %console_error_message = call ptr @thaw_error_console_inspect(ptr @strlit.16)
  %console_display = call ptr @thaw_string_to_display(ptr %console_error_message)
  call void @thaw_console_write(ptr %console_display, i32 1, i1 true)
  %fflush_call = call i32 @fflush(ptr null)
  %resolve_completion = call i8 @thaw_promise_resolve(ptr %completion, ptr %0)
  ret void
}

define internal void @__thaw_lambda_0(ptr %0, ptr %1) #1 {
entry:
  %r_cell2 = call ptr @thaw_arena_alloc(i64 ptrtoint (ptr getelementptr (ptr, ptr null, i32 1) to i64), i64 8)
  %stack_probe_cell = alloca i8, align 1
  %r_cell = alloca ptr, align 8
  store ptr %1, ptr %r_cell, align 8
  %stack_pointer = ptrtoint ptr %stack_probe_cell to i64
  %stack_limit = load i64, ptr @__thaw_stack_limit, align 4
  %stack_overflow = icmp ult i64 %stack_pointer, %stack_limit
  br i1 %stack_overflow, label %stack_overflow1, label %stack_ok, !prof !0

stack_overflow1:                                  ; preds = %entry
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 4, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  store ptr @strlit.4, ptr @__thaw_pending_exception_native_text, align 8
  store ptr @strlit.4, ptr @__thaw_pending_exception, align 8
  ret void

stack_ok:                                         ; preds = %entry
  %captured_stack_value = load ptr, ptr %r_cell, align 8
  store ptr %captured_stack_value, ptr %r_cell2, align 8
  %closure_alloc = call ptr @thaw_arena_alloc(i64 24, i64 8)
  store ptr @__thaw_lambda_1, ptr %closure_alloc, align 8
  %closure_this_entry = getelementptr inbounds i8, ptr %closure_alloc, i64 8
  store ptr @__thaw_lambda_1__thaw_this_adapter, ptr %closure_this_entry, align 8
  %capture_slot = getelementptr inbounds i8, ptr %closure_alloc, i64 16
  store ptr %r_cell2, ptr %capture_slot, align 8
  %call = call i64 @thaw_js_get_global(ptr @strlit.10)
  %callable_is_undefined = icmp eq ptr %closure_alloc, null
  br i1 %callable_is_undefined, label %undefined_callable, label %present_callable

undefined_callable:                               ; preds = %stack_ok
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret void

present_callable:                                 ; preds = %stack_ok
  %closure_code = load ptr, ptr %closure_alloc, align 8
  %closure_call = call ptr %closure_code(ptr %closure_alloc, i64 %call)
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

call_ok:                                          ; preds = %present_callable
  ret void

propagate_exception:                              ; preds = %present_callable
  ret void
}

define internal void @__thaw_lambda_0__thaw_this_adapter(ptr %0, { i8, i64 } %1, ptr %2) #1 {
entry:
  call void @__thaw_lambda_0(ptr %0, ptr %2)
  ret void
}

define internal ptr @__thaw_lambda_1(ptr %0, i64 %1) #1 {
entry:
  %__thaw_global_timer_0_cell = alloca i64, align 8
  %captured = getelementptr inbounds i8, ptr %0, i64 16
  %capture_cell = load ptr, ptr %captured, align 8
  store i64 %1, ptr %__thaw_global_timer_0_cell, align 4
  %__thaw_global_timer_0 = load i64, ptr %__thaw_global_timer_0_cell, align 4
  %closure_alloc = call ptr @thaw_arena_alloc(i64 16, i64 8)
  store ptr @__thaw_lambda_2, ptr %closure_alloc, align 8
  %closure_this_entry = getelementptr inbounds i8, ptr %closure_alloc, i64 8
  store ptr @__thaw_lambda_2__thaw_this_adapter, ptr %closure_this_entry, align 8
  %closure_alloc1 = call ptr @thaw_arena_alloc(i64 16, i64 8)
  store ptr @__thaw_lambda_4, ptr %closure_alloc1, align 8
  %closure_this_entry2 = getelementptr inbounds i8, ptr %closure_alloc1, i64 8
  store ptr @__thaw_lambda_4__thaw_this_adapter, ptr %closure_this_entry2, align 8
  %r = load ptr, ptr %capture_cell, align 8
  %native_callback_is_undefined = icmp eq ptr %r, null
  br i1 %native_callback_is_undefined, label %native_callback_absent_pointer, label %native_callback_present_pointer

native_callback_present_pointer:                  ; preds = %entry
  call void @thaw_json_register_host_operations(ptr @thaw_js_retain_handle, ptr @thaw_js_release_handle, ptr @thaw_js_get_property_json_key_result, ptr @thaw_js_host_query_result, ptr @thaw_js_host_date_set_result, ptr @thaw_js_set_property_graph_result, ptr @thaw_js_property_predicate_json_key_result, ptr @thaw_js_host_enumerate_result)
  call void @thaw_json_register_error_factory(ptr @thaw_js_error_from_tagged_handle, ptr @thaw_js_lookup_native_object, ptr @thaw_js_intern_native_object)
  %js_engine_id = call i8 @thaw_js_engine_id()
  call void @thaw_json_set_js_engine(i8 %js_engine_id)
  %register_native_callback = call { i64, ptr } @thaw_js_register_native_callback_graph(ptr @__thaw_callback_pending_guard_8, ptr %r, i64 0, i64 1, i8 1, ptr null, i8 0)
  %register_native_callback_value = extractvalue { i64, ptr } %register_native_callback, 0
  %register_native_callback_error = extractvalue { i64, ptr } %register_native_callback, 1
  store ptr %register_native_callback_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %register_native_callback_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

native_callback_absent_pointer:                   ; preds = %entry
  %native_callback_undefined_handle = call i64 @thaw_js_get_global(ptr @native_callback_undefined_name)
  br label %native_callback_pointer_done

native_callback_pointer_done:                     ; preds = %native_callback_absent_pointer, %call_ok
  %native_callback_handle = phi i64 [ %register_native_callback_value, %call_ok ], [ %native_callback_undefined_handle, %native_callback_absent_pointer ]
  %callable_is_undefined = icmp eq ptr %closure_alloc1, null
  br i1 %callable_is_undefined, label %undefined_callable, label %present_callable

call_ok:                                          ; preds = %native_callback_present_pointer
  br label %native_callback_pointer_done

propagate_exception:                              ; preds = %native_callback_present_pointer
  ret ptr null

undefined_callable:                               ; preds = %native_callback_pointer_done
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret ptr null

present_callable:                                 ; preds = %native_callback_pointer_done
  %closure_code = load ptr, ptr %closure_alloc1, align 8
  %closure_call = call ptr %closure_code(ptr %closure_alloc1, i64 %native_callback_handle)
  %pending_exception3 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception4 = icmp ne ptr %pending_exception3, null
  br i1 %has_pending_exception4, label %propagate_exception6, label %call_ok5

call_ok5:                                         ; preds = %present_callable
  %closure_alloc7 = call ptr @thaw_arena_alloc(i64 16, i64 8)
  store ptr @__thaw_lambda_9, ptr %closure_alloc7, align 8
  %closure_this_entry8 = getelementptr inbounds i8, ptr %closure_alloc7, i64 8
  store ptr @__thaw_lambda_9__thaw_this_adapter, ptr %closure_this_entry8, align 8
  %callable_is_undefined9 = icmp eq ptr %closure_alloc7, null
  br i1 %callable_is_undefined9, label %undefined_callable10, label %present_callable11

propagate_exception6:                             ; preds = %present_callable
  ret ptr null

undefined_callable10:                             ; preds = %call_ok5
  %synthetic_error_frame12 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret ptr null

present_callable11:                               ; preds = %call_ok5
  %closure_code13 = load ptr, ptr %closure_alloc7, align 8
  %closure_call14 = call ptr %closure_code13(ptr %closure_alloc7, double 1.000000e+01)
  %pending_exception15 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception16 = icmp ne ptr %pending_exception15, null
  br i1 %has_pending_exception16, label %propagate_exception18, label %call_ok17

call_ok17:                                        ; preds = %present_callable11
  %arr_alloc = call ptr @thaw_arena_alloc(i64 24, i64 8)
  store i64 2, ptr %arr_alloc, align 4
  %elem_ptr = getelementptr inbounds i8, ptr %arr_alloc, i64 8
  store ptr %closure_call, ptr %elem_ptr, align 8
  %elem_ptr19 = getelementptr inbounds i8, ptr %arr_alloc, i64 16
  store ptr %closure_call14, ptr %elem_ptr19, align 8
  %array_handle = call ptr @thaw_arena_alloc(i64 16, i64 8)
  store ptr %arr_alloc, ptr %array_handle, align 8
  %array_presence_slot = getelementptr inbounds i8, ptr %array_handle, i64 8
  store ptr null, ptr %array_presence_slot, align 8
  %callable_is_undefined20 = icmp eq ptr %closure_alloc, null
  br i1 %callable_is_undefined20, label %undefined_callable21, label %present_callable22

propagate_exception18:                            ; preds = %present_callable11
  ret ptr null

undefined_callable21:                             ; preds = %call_ok17
  %synthetic_error_frame23 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret ptr null

present_callable22:                               ; preds = %call_ok17
  %closure_code24 = load ptr, ptr %closure_alloc, align 8
  %closure_call25 = call ptr %closure_code24(ptr %closure_alloc, ptr %array_handle)
  %pending_exception26 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception27 = icmp ne ptr %pending_exception26, null
  br i1 %has_pending_exception27, label %propagate_exception29, label %call_ok28

call_ok28:                                        ; preds = %present_callable22
  %call_dynamic_args_json = call ptr @thaw_json_graph_encode(ptr %closure_call25)
  %json_stringify_error = call i8 @thaw_json_take_stringify_error()
  %json_stringify_failed = icmp ne i8 %json_stringify_error, 0
  br i1 %json_stringify_failed, label %json_stringify_invalid, label %json_stringify_valid

propagate_exception29:                            ; preds = %present_callable22
  ret ptr null

json_stringify_invalid:                           ; preds = %call_ok28
  call void @thaw_cstring_destroy(ptr %call_dynamic_args_json)
  call void @thaw_json_destroy(ptr %closure_call25)
  %json_stringify_error_message = call ptr @thaw_json_stringify_error_message()
  %synthetic_error_frame30 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr %json_stringify_error_message)
  ret ptr null

json_stringify_valid:                             ; preds = %call_ok28
  %json_stringify_host_error = call ptr @thaw_json_take_host_error()
  %json_stringify_host_failed31 = icmp ne ptr %json_stringify_host_error, null
  br i1 %json_stringify_host_failed31, label %json_stringify_host_failed, label %json_stringify_host_valid

json_stringify_host_failed:                       ; preds = %json_stringify_valid
  call void @thaw_cstring_destroy(ptr %call_dynamic_args_json)
  call void @thaw_json_destroy(ptr %closure_call25)
  store ptr %json_stringify_host_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_stringify_host_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception32 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception33 = icmp ne ptr %pending_exception32, null
  br i1 %has_pending_exception33, label %propagate_exception35, label %call_ok34

json_stringify_host_valid:                        ; preds = %call_ok34, %json_stringify_valid
  %call_dynamic_result_abi = call { ptr, ptr } @thaw_js_call_handle_graph_result(i64 %__thaw_global_timer_0, ptr %call_dynamic_args_json)
  %call_dynamic_value = extractvalue { ptr, ptr } %call_dynamic_result_abi, 0
  %call_dynamic_error = extractvalue { ptr, ptr } %call_dynamic_result_abi, 1
  call void @thaw_cstring_destroy(ptr %call_dynamic_args_json)
  call void @thaw_json_destroy(ptr %closure_call25)
  store ptr %call_dynamic_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %call_dynamic_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception36 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception37 = icmp ne ptr %pending_exception36, null
  br i1 %has_pending_exception37, label %propagate_exception39, label %call_ok38

call_ok34:                                        ; preds = %json_stringify_host_failed
  br label %json_stringify_host_valid

propagate_exception35:                            ; preds = %json_stringify_host_failed
  ret ptr null

call_ok38:                                        ; preds = %json_stringify_host_valid
  %decode_quickjs_graph = call ptr @thaw_json_graph_decode(ptr %call_dynamic_value)
  call void @thaw_cstring_destroy(ptr %call_dynamic_value)
  %quickjs_graph_error = call i8 @thaw_json_take_graph_error()
  %quickjs_graph_failed = icmp ne i8 %quickjs_graph_error, 0
  br i1 %quickjs_graph_failed, label %quickjs_graph_invalid, label %quickjs_graph_valid

propagate_exception39:                            ; preds = %json_stringify_host_valid
  ret ptr null

quickjs_graph_invalid:                            ; preds = %call_ok38
  call void @thaw_json_destroy(ptr %decode_quickjs_graph)
  %synthetic_error_frame40 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.8)
  ret ptr null

quickjs_graph_valid:                              ; preds = %call_ok38
  ret ptr %decode_quickjs_graph
}

define internal ptr @__thaw_lambda_1__thaw_this_adapter(ptr %0, { i8, i64 } %1, i64 %2) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_1(ptr %0, i64 %2)
  ret ptr %invoke_ignoring_this
}

define internal ptr @__thaw_lambda_2(ptr %0, ptr %1) #1 {
entry:
  %__thaw_json_wrap_value_6_cell1 = call ptr @thaw_arena_alloc(i64 ptrtoint (ptr getelementptr (ptr, ptr null, i32 1) to i64), i64 8)
  %__thaw_json_wrap_value_6_cell = alloca ptr, align 8
  store ptr %1, ptr %__thaw_json_wrap_value_6_cell, align 8
  %captured_stack_value = load ptr, ptr %__thaw_json_wrap_value_6_cell, align 8
  store ptr %captured_stack_value, ptr %__thaw_json_wrap_value_6_cell1, align 8
  %closure_alloc = call ptr @thaw_arena_alloc(i64 24, i64 8)
  store ptr @__thaw_lambda_3, ptr %closure_alloc, align 8
  %closure_this_entry = getelementptr inbounds i8, ptr %closure_alloc, i64 8
  store ptr @__thaw_lambda_3__thaw_this_adapter, ptr %closure_this_entry, align 8
  %capture_slot = getelementptr inbounds i8, ptr %closure_alloc, i64 16
  store ptr %__thaw_json_wrap_value_6_cell1, ptr %capture_slot, align 8
  %callable_is_undefined = icmp eq ptr %closure_alloc, null
  br i1 %callable_is_undefined, label %undefined_callable, label %present_callable

undefined_callable:                               ; preds = %entry
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret ptr null

present_callable:                                 ; preds = %entry
  %closure_code = load ptr, ptr %closure_alloc, align 8
  %closure_call = call ptr %closure_code(ptr %closure_alloc)
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

call_ok:                                          ; preds = %present_callable
  ret ptr %closure_call

propagate_exception:                              ; preds = %present_callable
  ret ptr null
}

define internal ptr @__thaw_lambda_2__thaw_this_adapter(ptr %0, { i8, i64 } %1, ptr %2) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_2(ptr %0, ptr %2)
  ret ptr %invoke_ignoring_this
}

define internal ptr @__thaw_lambda_3(ptr %0) #1 {
entry:
  %owned_json_transferred = alloca i1, align 1
  %__thaw_json_wrap_obj_7_cell = alloca ptr, align 8
  %stack_probe_cell = alloca i8, align 1
  %captured = getelementptr inbounds i8, ptr %0, i64 16
  %capture_cell = load ptr, ptr %captured, align 8
  %stack_pointer = ptrtoint ptr %stack_probe_cell to i64
  %stack_limit = load i64, ptr @__thaw_stack_limit, align 4
  %stack_overflow = icmp ult i64 %stack_pointer, %stack_limit
  br i1 %stack_overflow, label %stack_overflow1, label %stack_ok, !prof !0

stack_overflow1:                                  ; preds = %entry
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 4, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  store ptr @strlit.4, ptr @__thaw_pending_exception_native_text, align 8
  store ptr @strlit.4, ptr @__thaw_pending_exception, align 8
  ret ptr null

stack_ok:                                         ; preds = %entry
  %dictionary = call ptr @thaw_json_object_new()
  store ptr %dictionary, ptr %__thaw_json_wrap_obj_7_cell, align 8
  %__thaw_json_wrap_obj_7 = load ptr, ptr %__thaw_json_wrap_obj_7_cell, align 8
  %__thaw_json_wrap_value_6 = load ptr, ptr %capture_cell, align 8
  %array_data = load ptr, ptr %__thaw_json_wrap_value_6, align 8
  %console_array_json = call ptr @thaw_json_array_new()
  call void @thaw_json_typed_decode_scope_own(ptr %console_array_json)
  %console_array_length = load i64, ptr %array_data, align 4
  br label %console_array_next

console_array_next:                               ; preds = %console_array_pushed, %stack_ok
  %console_array_index = phi i64 [ 0, %stack_ok ], [ %console_array_increment, %console_array_pushed ]
  %console_array_has_element = icmp ult i64 %console_array_index, %console_array_length
  br i1 %console_array_has_element, label %console_array_element, label %console_array_done

console_array_element:                            ; preds = %console_array_next
  %array_data2 = load ptr, ptr %__thaw_json_wrap_value_6, align 8
  %array_current_length = load i64, ptr %array_data2, align 4
  %array_index_in_current_length = icmp ult i64 %console_array_index, %array_current_length
  br i1 %array_index_in_current_length, label %array_check_presence, label %array_out_of_range

console_array_done:                               ; preds = %console_array_next
  store i1 false, ptr %owned_json_transferred, align 1
  call void @thaw_json_object_set_json_owned(ptr %__thaw_json_wrap_obj_7, ptr @strlit.5, ptr %console_array_json)
  %json_host_error = call ptr @thaw_json_take_host_error()
  %json_host_has_error = icmp ne ptr %json_host_error, null
  br i1 %json_host_has_error, label %json_host_failed, label %json_host_succeeded

array_check_presence:                             ; preds = %console_array_element
  %array_presence_slot = getelementptr inbounds i8, ptr %__thaw_json_wrap_value_6, i64 8
  %array_presence = load ptr, ptr %array_presence_slot, align 8
  %array_is_dense = icmp eq ptr %array_presence, null
  br i1 %array_is_dense, label %array_dense, label %array_sparse

array_out_of_range:                               ; preds = %console_array_element
  br label %array_presence_done

array_dense:                                      ; preds = %array_check_presence
  br label %array_presence_done

array_sparse:                                     ; preds = %array_check_presence
  %array_presence_length = load i64, ptr %array_presence, align 4
  %array_index_in_presence_mask = icmp ult i64 %console_array_index, %array_presence_length
  br i1 %array_index_in_presence_mask, label %array_sparse_present, label %array_beyond_presence_mask

array_sparse_present:                             ; preds = %array_sparse
  %array_presence_element = getelementptr inbounds i8, ptr %array_presence, i64 %console_array_index
  %array_presence_payload = getelementptr inbounds i8, ptr %array_presence_element, i64 8
  %array_element_state = load i8, ptr %array_presence_payload, align 1
  br label %array_presence_done

array_beyond_presence_mask:                       ; preds = %array_sparse
  br label %array_presence_done

array_presence_done:                              ; preds = %array_out_of_range, %array_beyond_presence_mask, %array_sparse_present, %array_dense
  %array_index_state = phi i8 [ 0, %array_out_of_range ], [ 1, %array_dense ], [ %array_element_state, %array_sparse_present ], [ 1, %array_beyond_presence_mask ]
  %console_array_has_value = icmp eq i8 %array_index_state, 1
  br i1 %console_array_has_value, label %console_array_present, label %console_array_hole

console_array_present:                            ; preds = %array_presence_done
  %console_array_element_offset = mul i64 %console_array_index, 8
  %console_array_payload_offset = add i64 %console_array_element_offset, 8
  %console_array_element_pointer = getelementptr inbounds i8, ptr %array_data, i64 %console_array_payload_offset
  %console_array_element_value = load ptr, ptr %console_array_element_pointer, align 8
  call void @thaw_json_array_push_json(ptr %console_array_json, ptr %console_array_element_value)
  br label %console_array_pushed

console_array_hole:                               ; preds = %array_presence_done
  %console_array_is_hole = icmp eq i8 %array_index_state, 0
  br i1 %console_array_is_hole, label %console_array_absent, label %console_array_explicit_undefined

console_array_pushed:                             ; preds = %console_array_explicit_undefined, %console_array_absent, %console_array_present
  %console_array_increment = add i64 %console_array_index, 1
  br label %console_array_next

console_array_absent:                             ; preds = %console_array_hole
  call void @thaw_json_array_push_hole(ptr %console_array_json)
  br label %console_array_pushed

console_array_explicit_undefined:                 ; preds = %console_array_hole
  %napi_undefined_json = call ptr @thaw_json_object_new()
  call void @thaw_json_object_set_bool(ptr %napi_undefined_json, ptr @napi_undefined_key, i8 1)
  %brand_napi_undefined = call ptr @thaw_json_brand_wrapper(ptr %napi_undefined_json)
  call void @thaw_json_array_push_json(ptr %console_array_json, ptr %brand_napi_undefined)
  call void @thaw_json_destroy(ptr %brand_napi_undefined)
  br label %console_array_pushed

json_host_failed:                                 ; preds = %console_array_done
  store ptr %json_host_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

json_host_succeeded:                              ; preds = %call_ok, %console_array_done
  store i1 true, ptr %owned_json_transferred, align 1
  %__thaw_json_wrap_obj_73 = load ptr, ptr %__thaw_json_wrap_obj_7_cell, align 8
  %json_get = call ptr @thaw_json_take(ptr %__thaw_json_wrap_obj_73, ptr @strlit.5)
  %json_host_error4 = call ptr @thaw_json_take_host_error()
  %json_host_has_error7 = icmp ne ptr %json_host_error4, null
  br i1 %json_host_has_error7, label %json_host_failed5, label %json_host_succeeded6

call_ok:                                          ; preds = %json_host_failed
  br label %json_host_succeeded

propagate_exception:                              ; preds = %json_host_failed
  ret ptr null

json_host_failed5:                                ; preds = %json_host_succeeded
  call void @thaw_json_destroy(ptr %json_get)
  store ptr %json_host_error4, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error4, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception8 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception9 = icmp ne ptr %pending_exception8, null
  br i1 %has_pending_exception9, label %propagate_exception11, label %call_ok10

json_host_succeeded6:                             ; preds = %call_ok10, %json_host_succeeded
  ret ptr %json_get

call_ok10:                                        ; preds = %json_host_failed5
  br label %json_host_succeeded6

propagate_exception11:                            ; preds = %json_host_failed5
  ret ptr null
}

define internal ptr @__thaw_lambda_3__thaw_this_adapter(ptr %0, { i8, i64 } %1) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_3(ptr %0)
  ret ptr %invoke_ignoring_this
}

define internal ptr @__thaw_lambda_4(ptr %0, i64 %1) #1 {
entry:
  %owned_json_transferred32 = alloca i1, align 1
  %owned_json_transferred = alloca i1, align 1
  %__thaw_json_wrap_source_1_cell = alloca i64, align 8
  store i64 %1, ptr %__thaw_json_wrap_source_1_cell, align 4
  %call = call i64 @thaw_js_get_global(ptr @strlit.7)
  %__thaw_json_wrap_source_1 = load i64, ptr %__thaw_json_wrap_source_1_cell, align 4
  %dynamic_value_argument_result = call { ptr, ptr } @thaw_js_call_handle_value_graph_result(i64 %call, i64 %__thaw_json_wrap_source_1)
  %dynamic_value_argument_json = extractvalue { ptr, ptr } %dynamic_value_argument_result, 0
  %dynamic_value_argument_error = extractvalue { ptr, ptr } %dynamic_value_argument_result, 1
  store ptr %dynamic_value_argument_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %dynamic_value_argument_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %cleanup_before_catch, label %call_ok

conditional_then:                                 ; preds = %json_host_succeeded
  %closure_alloc = call ptr @thaw_arena_alloc(i64 16, i64 8)
  store ptr @__thaw_lambda_5, ptr %closure_alloc, align 8
  %closure_this_entry = getelementptr inbounds i8, ptr %closure_alloc, i64 8
  store ptr @__thaw_lambda_5__thaw_this_adapter, ptr %closure_this_entry, align 8
  %callable_is_undefined = icmp eq ptr %closure_alloc, null
  br i1 %callable_is_undefined, label %undefined_callable, label %present_callable

conditional_else:                                 ; preds = %json_host_succeeded
  %call16 = call i64 @thaw_js_get_global(ptr @strlit.9)
  %__thaw_json_wrap_source_118 = load i64, ptr %__thaw_json_wrap_source_1_cell, align 4
  %dynamic_value_argument_result19 = call { ptr, ptr } @thaw_js_call_handle_value_graph_result(i64 %call16, i64 %__thaw_json_wrap_source_118)
  %dynamic_value_argument_json20 = extractvalue { ptr, ptr } %dynamic_value_argument_result19, 0
  %dynamic_value_argument_error21 = extractvalue { ptr, ptr } %dynamic_value_argument_result19, 1
  store ptr %dynamic_value_argument_error21, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %dynamic_value_argument_error21, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception22 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception23 = icmp ne ptr %pending_exception22, null
  br i1 %has_pending_exception23, label %cleanup_before_catch25, label %call_ok24

conditional_end:                                  ; preds = %conditional_end15, %call_ok11
  %conditional_value72 = phi ptr [ %closure_call, %call_ok11 ], [ %conditional_value, %conditional_end15 ]
  ret ptr %conditional_value72

dynamic_value_argument_callee_cleanup:            ; preds = %quickjs_graph_invalid, %cleanup_before_catch
  %release_inline_dynamic_callee_on_error = call i8 @thaw_js_release_handle(i64 %call)
  %pending_exception1 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception2 = icmp ne ptr %pending_exception1, null
  br i1 %has_pending_exception2, label %propagate_exception, label %call_ok3

call_ok:                                          ; preds = %entry
  %decode_quickjs_graph = call ptr @thaw_json_graph_decode(ptr %dynamic_value_argument_json)
  call void @thaw_cstring_destroy(ptr %dynamic_value_argument_json)
  %quickjs_graph_error = call i8 @thaw_json_take_graph_error()
  %quickjs_graph_failed = icmp ne i8 %quickjs_graph_error, 0
  br i1 %quickjs_graph_failed, label %quickjs_graph_invalid, label %quickjs_graph_valid

cleanup_before_catch:                             ; preds = %entry
  br label %dynamic_value_argument_callee_cleanup

quickjs_graph_invalid:                            ; preds = %call_ok
  call void @thaw_json_destroy(ptr %decode_quickjs_graph)
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.8)
  br label %dynamic_value_argument_callee_cleanup

quickjs_graph_valid:                              ; preds = %call_ok
  store i1 false, ptr %owned_json_transferred, align 1
  %release_inline_dynamic_callee_after_call = call i8 @thaw_js_release_handle(i64 %call)
  %json_as_bool_u8 = call i8 @thaw_json_as_bool(ptr %decode_quickjs_graph)
  %operand_json_was_transferred = load i1, ptr %owned_json_transferred, align 1
  %operand_json_still_owned = xor i1 %operand_json_was_transferred, true
  %operand_json_all_owned = and i1 true, %operand_json_still_owned
  br i1 %operand_json_all_owned, label %destroy_consumed_json_operand, label %consumed_json_operand_done

call_ok3:                                         ; preds = %dynamic_value_argument_callee_cleanup
  unreachable

propagate_exception:                              ; preds = %dynamic_value_argument_callee_cleanup
  ret ptr null

destroy_consumed_json_operand:                    ; preds = %quickjs_graph_valid
  call void @thaw_json_destroy(ptr %decode_quickjs_graph)
  store i1 true, ptr %owned_json_transferred, align 1
  br label %consumed_json_operand_done

consumed_json_operand_done:                       ; preds = %destroy_consumed_json_operand, %quickjs_graph_valid
  %json_host_error = call ptr @thaw_json_take_host_error()
  %json_host_has_error = icmp ne ptr %json_host_error, null
  br i1 %json_host_has_error, label %json_host_failed, label %json_host_succeeded

json_host_failed:                                 ; preds = %consumed_json_operand_done
  store ptr %json_host_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception4 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception5 = icmp ne ptr %pending_exception4, null
  br i1 %has_pending_exception5, label %propagate_exception7, label %call_ok6

json_host_succeeded:                              ; preds = %call_ok6, %consumed_json_operand_done
  %json_as_bool = icmp ne i8 %json_as_bool_u8, 0
  br i1 %json_as_bool, label %conditional_then, label %conditional_else

call_ok6:                                         ; preds = %json_host_failed
  br label %json_host_succeeded

propagate_exception7:                             ; preds = %json_host_failed
  ret ptr null

undefined_callable:                               ; preds = %conditional_then
  %synthetic_error_frame8 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret ptr null

present_callable:                                 ; preds = %conditional_then
  %closure_code = load ptr, ptr %closure_alloc, align 8
  %closure_call = call ptr %closure_code(ptr %closure_alloc, i1 true)
  %pending_exception9 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception10 = icmp ne ptr %pending_exception9, null
  br i1 %has_pending_exception10, label %propagate_exception12, label %call_ok11

call_ok11:                                        ; preds = %present_callable
  br label %conditional_end

propagate_exception12:                            ; preds = %present_callable
  ret ptr null

conditional_then13:                               ; preds = %json_host_succeeded47
  %dictionary = call ptr @thaw_json_object_new()
  call void @thaw_json_object_set_bool(ptr %dictionary, ptr @napi_undefined_key, i8 1)
  %json_host_error54 = call ptr @thaw_json_take_host_error()
  %json_host_has_error57 = icmp ne ptr %json_host_error54, null
  br i1 %json_host_has_error57, label %json_host_failed55, label %json_host_succeeded56

conditional_else14:                               ; preds = %json_host_succeeded47
  %__thaw_json_wrap_source_163 = load i64, ptr %__thaw_json_wrap_source_1_cell, align 4
  %js_value_as_json = call ptr @thaw_json_from_borrowed_handle(i64 %__thaw_json_wrap_source_163)
  %json_host_error64 = call ptr @thaw_json_take_host_error()
  %json_host_has_error67 = icmp ne ptr %json_host_error64, null
  br i1 %json_host_has_error67, label %json_host_failed65, label %json_host_succeeded66

conditional_end15:                                ; preds = %json_host_succeeded66, %json_host_succeeded56
  %conditional_value = phi ptr [ %call62, %json_host_succeeded56 ], [ %js_value_as_json, %json_host_succeeded66 ]
  br label %conditional_end

dynamic_value_argument_callee_cleanup17:          ; preds = %quickjs_graph_invalid29, %cleanup_before_catch25
  %release_inline_dynamic_callee_on_error33 = call i8 @thaw_js_release_handle(i64 %call16)
  %pending_exception34 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception35 = icmp ne ptr %pending_exception34, null
  br i1 %has_pending_exception35, label %propagate_exception37, label %call_ok36

call_ok24:                                        ; preds = %conditional_else
  %decode_quickjs_graph26 = call ptr @thaw_json_graph_decode(ptr %dynamic_value_argument_json20)
  call void @thaw_cstring_destroy(ptr %dynamic_value_argument_json20)
  %quickjs_graph_error27 = call i8 @thaw_json_take_graph_error()
  %quickjs_graph_failed28 = icmp ne i8 %quickjs_graph_error27, 0
  br i1 %quickjs_graph_failed28, label %quickjs_graph_invalid29, label %quickjs_graph_valid30

cleanup_before_catch25:                           ; preds = %conditional_else
  br label %dynamic_value_argument_callee_cleanup17

quickjs_graph_invalid29:                          ; preds = %call_ok24
  call void @thaw_json_destroy(ptr %decode_quickjs_graph26)
  %synthetic_error_frame31 = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.8)
  br label %dynamic_value_argument_callee_cleanup17

quickjs_graph_valid30:                            ; preds = %call_ok24
  store i1 false, ptr %owned_json_transferred32, align 1
  %release_inline_dynamic_callee_after_call38 = call i8 @thaw_js_release_handle(i64 %call16)
  %json_as_bool_u839 = call i8 @thaw_json_as_bool(ptr %decode_quickjs_graph26)
  %operand_json_was_transferred40 = load i1, ptr %owned_json_transferred32, align 1
  %operand_json_still_owned41 = xor i1 %operand_json_was_transferred40, true
  %operand_json_all_owned42 = and i1 true, %operand_json_still_owned41
  br i1 %operand_json_all_owned42, label %destroy_consumed_json_operand43, label %consumed_json_operand_done44

call_ok36:                                        ; preds = %dynamic_value_argument_callee_cleanup17
  unreachable

propagate_exception37:                            ; preds = %dynamic_value_argument_callee_cleanup17
  ret ptr null

destroy_consumed_json_operand43:                  ; preds = %quickjs_graph_valid30
  call void @thaw_json_destroy(ptr %decode_quickjs_graph26)
  store i1 true, ptr %owned_json_transferred32, align 1
  br label %consumed_json_operand_done44

consumed_json_operand_done44:                     ; preds = %destroy_consumed_json_operand43, %quickjs_graph_valid30
  %json_host_error45 = call ptr @thaw_json_take_host_error()
  %json_host_has_error48 = icmp ne ptr %json_host_error45, null
  br i1 %json_host_has_error48, label %json_host_failed46, label %json_host_succeeded47

json_host_failed46:                               ; preds = %consumed_json_operand_done44
  store ptr %json_host_error45, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error45, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception49 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception50 = icmp ne ptr %pending_exception49, null
  br i1 %has_pending_exception50, label %propagate_exception52, label %call_ok51

json_host_succeeded47:                            ; preds = %call_ok51, %consumed_json_operand_done44
  %json_as_bool53 = icmp ne i8 %json_as_bool_u839, 0
  br i1 %json_as_bool53, label %conditional_then13, label %conditional_else14

call_ok51:                                        ; preds = %json_host_failed46
  br label %json_host_succeeded47

propagate_exception52:                            ; preds = %json_host_failed46
  ret ptr null

json_host_failed55:                               ; preds = %conditional_then13
  store ptr %json_host_error54, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error54, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception58 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception59 = icmp ne ptr %pending_exception58, null
  br i1 %has_pending_exception59, label %propagate_exception61, label %call_ok60

json_host_succeeded56:                            ; preds = %call_ok60, %conditional_then13
  %call62 = call ptr @thaw_json_brand_wrapper(ptr %dictionary)
  br label %conditional_end15

call_ok60:                                        ; preds = %json_host_failed55
  br label %json_host_succeeded56

propagate_exception61:                            ; preds = %json_host_failed55
  ret ptr null

json_host_failed65:                               ; preds = %conditional_else14
  call void @thaw_json_destroy(ptr %js_value_as_json)
  store ptr %json_host_error64, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error64, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception68 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception69 = icmp ne ptr %pending_exception68, null
  br i1 %has_pending_exception69, label %propagate_exception71, label %call_ok70

json_host_succeeded66:                            ; preds = %call_ok70, %conditional_else14
  br label %conditional_end15

call_ok70:                                        ; preds = %json_host_failed65
  br label %json_host_succeeded66

propagate_exception71:                            ; preds = %json_host_failed65
  ret ptr null
}

define internal ptr @__thaw_lambda_4__thaw_this_adapter(ptr %0, { i8, i64 } %1, i64 %2) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_4(ptr %0, i64 %2)
  ret ptr %invoke_ignoring_this
}

define internal ptr @__thaw_lambda_5(ptr %0, i1 %1) #1 {
entry:
  %__thaw_json_wrap_value_2_cell1 = call ptr @thaw_arena_alloc(i64 ptrtoint (ptr getelementptr (i1, ptr null, i32 1) to i64), i64 8)
  %__thaw_json_wrap_value_2_cell = alloca i1, align 1
  store i1 %1, ptr %__thaw_json_wrap_value_2_cell, align 1
  %captured_stack_value = load i1, ptr %__thaw_json_wrap_value_2_cell, align 1
  store i1 %captured_stack_value, ptr %__thaw_json_wrap_value_2_cell1, align 1
  %closure_alloc = call ptr @thaw_arena_alloc(i64 24, i64 8)
  store ptr @__thaw_lambda_6, ptr %closure_alloc, align 8
  %closure_this_entry = getelementptr inbounds i8, ptr %closure_alloc, i64 8
  store ptr @__thaw_lambda_6__thaw_this_adapter, ptr %closure_this_entry, align 8
  %capture_slot = getelementptr inbounds i8, ptr %closure_alloc, i64 16
  store ptr %__thaw_json_wrap_value_2_cell1, ptr %capture_slot, align 8
  %callable_is_undefined = icmp eq ptr %closure_alloc, null
  br i1 %callable_is_undefined, label %undefined_callable, label %present_callable

undefined_callable:                               ; preds = %entry
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret ptr null

present_callable:                                 ; preds = %entry
  %closure_code = load ptr, ptr %closure_alloc, align 8
  %closure_call = call ptr %closure_code(ptr %closure_alloc)
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

call_ok:                                          ; preds = %present_callable
  ret ptr %closure_call

propagate_exception:                              ; preds = %present_callable
  ret ptr null
}

define internal ptr @__thaw_lambda_5__thaw_this_adapter(ptr %0, { i8, i64 } %1, i1 %2) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_5(ptr %0, i1 %2)
  ret ptr %invoke_ignoring_this
}

define internal ptr @__thaw_lambda_6(ptr %0) #1 {
entry:
  %__thaw_json_wrap_obj_3_cell = alloca ptr, align 8
  %stack_probe_cell = alloca i8, align 1
  %captured = getelementptr inbounds i8, ptr %0, i64 16
  %capture_cell = load ptr, ptr %captured, align 8
  %stack_pointer = ptrtoint ptr %stack_probe_cell to i64
  %stack_limit = load i64, ptr @__thaw_stack_limit, align 4
  %stack_overflow = icmp ult i64 %stack_pointer, %stack_limit
  br i1 %stack_overflow, label %stack_overflow1, label %stack_ok, !prof !0

stack_overflow1:                                  ; preds = %entry
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 4, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  store ptr @strlit.4, ptr @__thaw_pending_exception_native_text, align 8
  store ptr @strlit.4, ptr @__thaw_pending_exception, align 8
  ret ptr null

stack_ok:                                         ; preds = %entry
  %dictionary = call ptr @thaw_json_object_new()
  store ptr %dictionary, ptr %__thaw_json_wrap_obj_3_cell, align 8
  %__thaw_json_wrap_obj_3 = load ptr, ptr %__thaw_json_wrap_obj_3_cell, align 8
  %__thaw_json_wrap_value_2 = load i1, ptr %capture_cell, align 1
  %json_null = call ptr @thaw_json_null()
  call void @thaw_json_object_set_json_owned(ptr %__thaw_json_wrap_obj_3, ptr @strlit.5, ptr %json_null)
  %json_host_error = call ptr @thaw_json_take_host_error()
  %json_host_has_error = icmp ne ptr %json_host_error, null
  br i1 %json_host_has_error, label %json_host_failed, label %json_host_succeeded

json_host_failed:                                 ; preds = %stack_ok
  store ptr %json_host_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

json_host_succeeded:                              ; preds = %call_ok, %stack_ok
  %__thaw_json_wrap_obj_32 = load ptr, ptr %__thaw_json_wrap_obj_3_cell, align 8
  %json_get = call ptr @thaw_json_take(ptr %__thaw_json_wrap_obj_32, ptr @strlit.5)
  %json_host_error3 = call ptr @thaw_json_take_host_error()
  %json_host_has_error6 = icmp ne ptr %json_host_error3, null
  br i1 %json_host_has_error6, label %json_host_failed4, label %json_host_succeeded5

call_ok:                                          ; preds = %json_host_failed
  br label %json_host_succeeded

propagate_exception:                              ; preds = %json_host_failed
  ret ptr null

json_host_failed4:                                ; preds = %json_host_succeeded
  call void @thaw_json_destroy(ptr %json_get)
  store ptr %json_host_error3, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error3, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception7 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception8 = icmp ne ptr %pending_exception7, null
  br i1 %has_pending_exception8, label %propagate_exception10, label %call_ok9

json_host_succeeded5:                             ; preds = %call_ok9, %json_host_succeeded
  ret ptr %json_get

call_ok9:                                         ; preds = %json_host_failed4
  br label %json_host_succeeded5

propagate_exception10:                            ; preds = %json_host_failed4
  ret ptr null
}

define internal ptr @__thaw_lambda_6__thaw_this_adapter(ptr %0, { i8, i64 } %1) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_6(ptr %0)
  ret ptr %invoke_ignoring_this
}

define internal ptr @__thaw_napi_value_callback_7(ptr %0, ptr %1) #1 {
entry:
  %callback_result_array_slot = alloca ptr, align 8
  %callback_projected_method_slot = alloca i64, align 8
  %callback_argument_json_slot = alloca ptr, align 8
  %callback_argument_json_slot1 = alloca ptr, align 8
  %parked_native_throw_snapshot_cell = alloca [9 x i64], align 8
  %napi_value_callback_args = call ptr @thaw_json_graph_decode(ptr %1)
  %native_callback_graph_error = call i8 @thaw_json_take_graph_error()
  %invalid_callback_graph = icmp ne i8 %native_callback_graph_error, 0
  br i1 %invalid_callback_graph, label %native_callback_graph_invalid, label %native_callback_graph_valid

native_callback_graph_invalid:                    ; preds = %entry
  call void @thaw_json_destroy(ptr %napi_value_callback_args)
  %invalid_callback_graph_result = call ptr @thaw_json_callback_error(ptr @invalid_callback_graph_message)
  ret ptr %invalid_callback_graph_result

native_callback_graph_valid:                      ; preds = %entry
  %native_callback_holder = call ptr @thaw_json_index(ptr %napi_value_callback_args, double 0.000000e+00, ptr null)
  store ptr null, ptr %callback_result_array_slot, align 8
  store i64 0, ptr %callback_projected_method_slot, align 4
  store ptr null, ptr %callback_argument_json_slot, align 8
  store ptr null, ptr %callback_argument_json_slot1, align 8
  store ptr %native_callback_holder, ptr %callback_argument_json_slot, align 8
  call void @thaw_json_typed_decode_scope_begin()
  %napi_value_callback_argument = call ptr @thaw_json_index(ptr %napi_value_callback_args, double 1.000000e+00, ptr null)
  store ptr %napi_value_callback_argument, ptr %callback_argument_json_slot1, align 8
  %json_is_napi_undefined = call i8 @thaw_json_is_undefined(ptr %napi_value_callback_argument)
  %json_is_napi_undefined_bool = icmp ne i8 %json_is_napi_undefined, 0
  %json_optional_present = xor i1 %json_is_napi_undefined_bool, true
  br i1 %json_optional_present, label %json_tagged_value, label %json_tagged_absent

callback_failed:                                  ; preds = %callback_result_marshalling_failed, %restore_decode_exception, %napi_value_callback_invoked
  %callback_failed_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %parked_native_throw_token = call i64 @thaw_exception_native_token(ptr %callback_failed_exception)
  %parked_native_throw_present = icmp ne i64 %parked_native_throw_token, 0
  br i1 %parked_native_throw_present, label %check_parked_native_throw_live, label %parked_native_throw_checked

callback_conversion_failed:                       ; No predecessors!
  %decode_exception_state = load ptr, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception, align 8
  %decode_exception_state3 = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  %decode_exception_state4 = load ptr, ptr @__thaw_pending_exception_object, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  %decode_exception_state5 = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %decode_exception_state6 = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  %decode_exception_state7 = load double, ptr @__thaw_pending_exception_f64, align 8
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  %decode_exception_state8 = load i64, ptr @__thaw_pending_exception_i64, align 4
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  %decode_exception_state9 = load i1, ptr @__thaw_pending_exception_bool, align 1
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  call void @thaw_json_typed_decode_scope_end(i8 1)
  %cleanup_native_text = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %has_cleanup_native_text = icmp ne ptr %cleanup_native_text, null
  %cleanup_native_word = ptrtoint ptr %cleanup_native_text to i64
  %original_native_word = ptrtoint ptr %decode_exception_state3 to i64
  %new_cleanup_native_text = icmp ne i64 %cleanup_native_word, %original_native_word
  %dispose_cleanup_error = and i1 %has_cleanup_native_text, %new_cleanup_native_text
  br i1 %dispose_cleanup_error, label %release_cleanup_error, label %restore_decode_exception

callback_result_marshalling_failed:               ; No predecessors!
  %failed_callback_result_array = load ptr, ptr %callback_result_array_slot, align 8
  call void @thaw_json_destroy(ptr %failed_callback_result_array)
  br label %callback_failed

json_tagged_value:                                ; preds = %native_callback_graph_valid
  %foreign_promise_placeholder = call ptr @thaw_promise_new()
  %optional_with_payload = insertvalue { i1, ptr } { i1 true, ptr undef }, ptr %foreign_promise_placeholder, 1
  br label %json_tagged_done

json_tagged_absent:                               ; preds = %native_callback_graph_valid
  br label %json_tagged_done

json_tagged_done:                                 ; preds = %json_tagged_absent, %json_tagged_value
  %json_tagged_field = phi { i1, ptr } [ %optional_with_payload, %json_tagged_value ], [ zeroinitializer, %json_tagged_absent ]
  call void @thaw_json_typed_decode_scope_end(i8 0)
  %native_callback_this_entry_slot = getelementptr inbounds i8, ptr %0, i64 8
  %napi_value_callback_code = load ptr, ptr %native_callback_this_entry_slot, align 8
  %callback_holder_word = ptrtoint ptr %native_callback_holder to i64
  %callback_holder_receiver = insertvalue { i8, i64 } { i8 7, i64 undef }, i64 %callback_holder_word, 1
  %napi_value_callback_this_entry_missing = icmp eq ptr %napi_value_callback_code, null
  br i1 %napi_value_callback_this_entry_missing, label %napi_value_callback_plain_entry, label %napi_value_callback_this_entry

napi_value_callback_this_entry:                   ; preds = %json_tagged_done
  call void %napi_value_callback_code(ptr %0, { i8, i64 } %callback_holder_receiver, { i1, ptr } %json_tagged_field)
  br label %napi_value_callback_invoked

napi_value_callback_plain_entry:                  ; preds = %json_tagged_done
  %napi_value_callback_plain_code = load ptr, ptr %0, align 8
  call void %napi_value_callback_plain_code(ptr %0, { i1, ptr } %json_tagged_field)
  br label %napi_value_callback_invoked

napi_value_callback_invoked:                      ; preds = %napi_value_callback_plain_entry, %napi_value_callback_this_entry
  %callback_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %callback_failed2 = icmp ne ptr %callback_exception, null
  br i1 %callback_failed2, label %callback_failed, label %callback_succeeded

callback_succeeded:                               ; preds = %napi_value_callback_invoked
  %napi_undefined_json = call ptr @thaw_json_object_new()
  call void @thaw_json_object_set_bool(ptr %napi_undefined_json, ptr @napi_undefined_key, i8 1)
  %brand_napi_undefined = call ptr @thaw_json_brand_wrapper(ptr %napi_undefined_json)
  call void @thaw_json_destroy(ptr %native_callback_holder)
  call void @thaw_json_destroy(ptr %napi_value_callback_argument)
  call void @thaw_json_destroy(ptr %napi_value_callback_args)
  %napi_value_callback_result_string = call ptr @thaw_json_graph_encode(ptr %brand_napi_undefined)
  call void @thaw_json_destroy(ptr %brand_napi_undefined)
  %callback_stringify_error = call i8 @thaw_json_take_stringify_error()
  %callback_stringify_failed = icmp ne i8 %callback_stringify_error, 0
  br i1 %callback_stringify_failed, label %callback_stringify_invalid, label %callback_stringify_valid

release_cleanup_error:                            ; preds = %callback_conversion_failed
  call void @thaw_cstring_destroy(ptr %cleanup_native_text)
  br label %restore_decode_exception

restore_decode_exception:                         ; preds = %release_cleanup_error, %callback_conversion_failed
  store ptr %decode_exception_state, ptr @__thaw_pending_exception, align 8
  store ptr %decode_exception_state3, ptr @__thaw_pending_exception_native_text, align 8
  store ptr %decode_exception_state4, ptr @__thaw_pending_exception_object, align 8
  store ptr %decode_exception_state5, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 %decode_exception_state6, ptr @__thaw_pending_exception_value_tag, align 4
  store double %decode_exception_state7, ptr @__thaw_pending_exception_f64, align 8
  store i64 %decode_exception_state8, ptr @__thaw_pending_exception_i64, align 4
  store i1 %decode_exception_state9, ptr @__thaw_pending_exception_bool, align 1
  store ptr null, ptr %callback_argument_json_slot1, align 8
  br label %callback_failed

restore_parked_native_throw:                      ; preds = %materialize_parked_native_throw
  %parked_native_throw_snapshot_field = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 0
  %parked_native_throw_field = load i64, ptr %parked_native_throw_snapshot_field, align 4
  %parked_native_throw_snapshot_field10 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 1
  %parked_native_throw_field11 = load i64, ptr %parked_native_throw_snapshot_field10, align 4
  %parked_native_throw_snapshot_field12 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 2
  %parked_native_throw_field13 = load i64, ptr %parked_native_throw_snapshot_field12, align 4
  %parked_native_throw_snapshot_field14 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 3
  %parked_native_throw_field15 = load i64, ptr %parked_native_throw_snapshot_field14, align 4
  %parked_native_throw_snapshot_field16 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 4
  %parked_native_throw_field17 = load i64, ptr %parked_native_throw_snapshot_field16, align 4
  %parked_native_throw_snapshot_field18 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 5
  %parked_native_throw_field19 = load i64, ptr %parked_native_throw_snapshot_field18, align 4
  %parked_native_throw_snapshot_field20 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 6
  %parked_native_throw_field21 = load i64, ptr %parked_native_throw_snapshot_field20, align 4
  %parked_native_throw_snapshot_field22 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 7
  %parked_native_throw_field23 = load i64, ptr %parked_native_throw_snapshot_field22, align 4
  %parked_native_throw_snapshot_field24 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 8
  %parked_native_throw_field25 = load i64, ptr %parked_native_throw_snapshot_field24, align 4
  %parked_native_throw_number = bitcast i64 %parked_native_throw_field11 to double
  %parked_native_throw_bool = trunc i64 %parked_native_throw_field15 to i1
  %parked_native_throw_object = inttoptr i64 %parked_native_throw_field17 to ptr
  %parked_native_throw_native = inttoptr i64 %parked_native_throw_field19 to ptr
  %parked_native_throw_pending = inttoptr i64 %parked_native_throw_field21 to ptr
  %parked_native_throw_native_text = inttoptr i64 %parked_native_throw_field23 to ptr
  %parked_native_throw_aggregate = inttoptr i64 %parked_native_throw_field25 to ptr
  store i64 %parked_native_throw_field, ptr @__thaw_pending_exception_value_tag, align 4
  store double %parked_native_throw_number, ptr @__thaw_pending_exception_f64, align 8
  store i64 %parked_native_throw_field13, ptr @__thaw_pending_exception_i64, align 4
  store i1 %parked_native_throw_bool, ptr @__thaw_pending_exception_bool, align 1
  store ptr %parked_native_throw_object, ptr @__thaw_pending_exception_object, align 8
  store ptr %parked_native_throw_native, ptr @__thaw_pending_exception_native, align 8
  store ptr %parked_native_throw_pending, ptr @__thaw_pending_exception, align 8
  store ptr %parked_native_throw_native_text, ptr @__thaw_pending_exception_native_text, align 8
  store ptr %parked_native_throw_aggregate, ptr @__thaw_pending_exception_aggregate_errors, align 8
  br label %parked_native_throw_checked

materialize_parked_native_throw:                  ; preds = %check_parked_native_throw_live
  %parked_native_throw_materialized = call i8 @thaw_exception_native_snapshot(i64 %parked_native_throw_token, ptr %parked_native_throw_snapshot_cell)
  %parked_native_throw_snapshot_ok = icmp ne i8 %parked_native_throw_materialized, 0
  br i1 %parked_native_throw_snapshot_ok, label %restore_parked_native_throw, label %parked_native_throw_checked

parked_native_throw_checked:                      ; preds = %restore_parked_native_throw, %materialize_parked_native_throw, %check_parked_native_throw_live, %callback_failed
  %callback_restored_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %callback_thrown_tag = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %callback_thrown_f64 = load double, ptr @__thaw_pending_exception_f64, align 8
  %callback_thrown_i64 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %callback_thrown_bool = load i1, ptr @__thaw_pending_exception_bool, align 1
  %callback_thrown_bool_byte = zext i1 %callback_thrown_bool to i8
  %callback_thrown_object = load ptr, ptr @__thaw_pending_exception_object, align 8
  %callback_thrown_native = load ptr, ptr @__thaw_pending_exception_native, align 8
  %callback_thrown_native_text = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %callback_thrown_aggregate_errors = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %callback_error_result = call ptr @thaw_json_callback_error_tuple(ptr %callback_restored_exception, i64 %callback_thrown_tag, double %callback_thrown_f64, i64 %callback_thrown_i64, i8 %callback_thrown_bool_byte, ptr %callback_thrown_object, ptr %callback_thrown_native, ptr %callback_thrown_native_text, ptr %callback_thrown_aggregate_errors)
  store ptr null, ptr @__thaw_pending_exception, align 8
  %failed_callback_json = load ptr, ptr %callback_argument_json_slot, align 8
  call void @thaw_json_destroy(ptr %failed_callback_json)
  %failed_callback_json26 = load ptr, ptr %callback_argument_json_slot1, align 8
  call void @thaw_json_destroy(ptr %failed_callback_json26)
  call void @thaw_json_destroy(ptr %napi_value_callback_args)
  ret ptr %callback_error_result

check_parked_native_throw_live:                   ; preds = %callback_failed
  %parked_native_throw_live = call i8 @thaw_exception_token_is_live(i64 %parked_native_throw_token)
  %parked_native_throw_is_live = icmp ne i8 %parked_native_throw_live, 0
  br i1 %parked_native_throw_is_live, label %materialize_parked_native_throw, label %parked_native_throw_checked

callback_stringify_invalid:                       ; preds = %callback_succeeded
  call void @thaw_cstring_destroy(ptr %napi_value_callback_result_string)
  %cyclic_callback_error = call ptr @thaw_json_callback_error(ptr @cyclic_callback_type_error)
  ret ptr %cyclic_callback_error

callback_stringify_valid:                         ; preds = %callback_succeeded
  %callback_stringify_host_error = call ptr @thaw_json_take_host_error()
  %callback_has_host_error = icmp ne ptr %callback_stringify_host_error, null
  br i1 %callback_has_host_error, label %callback_host_failed, label %callback_host_valid

callback_host_failed:                             ; preds = %callback_stringify_valid
  call void @thaw_cstring_destroy(ptr %napi_value_callback_result_string)
  %callback_host_error_result = call ptr @thaw_json_callback_error(ptr %callback_stringify_host_error)
  call void @thaw_cstring_destroy(ptr %callback_stringify_host_error)
  ret ptr %callback_host_error_result

callback_host_valid:                              ; preds = %callback_stringify_valid
  ret ptr %napi_value_callback_result_string
}

define internal ptr @__thaw_callback_pending_guard_8(ptr %0, ptr %1) #1 {
entry:
  %parked_native_throw_snapshot_cell = alloca [9 x i64], align 8
  %outer_exception_field = load ptr, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception, align 8
  %outer_exception_field1 = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  %outer_exception_field2 = load ptr, ptr @__thaw_pending_exception_object, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  %outer_exception_field3 = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %outer_exception_field4 = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  %outer_exception_field5 = load double, ptr @__thaw_pending_exception_f64, align 8
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  %outer_exception_field6 = load i64, ptr @__thaw_pending_exception_i64, align 4
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  %outer_exception_field7 = load i1, ptr @__thaw_pending_exception_bool, align 1
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  %run_isolated_host_callback = call ptr @__thaw_napi_value_callback_7(ptr %0, ptr %1)
  %own_callback_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %callback_has_own_exception = icmp ne ptr %own_callback_exception, null
  br i1 %callback_has_own_exception, label %callback_raised, label %callback_returned

callback_raised:                                  ; preds = %entry
  %parked_native_throw_token = call i64 @thaw_exception_native_token(ptr %own_callback_exception)
  %parked_native_throw_present = icmp ne i64 %parked_native_throw_token, 0
  br i1 %parked_native_throw_present, label %check_parked_native_throw_live, label %parked_native_throw_checked

callback_returned:                                ; preds = %entry
  store ptr %outer_exception_field, ptr @__thaw_pending_exception, align 8
  store ptr %outer_exception_field1, ptr @__thaw_pending_exception_native_text, align 8
  store ptr %outer_exception_field2, ptr @__thaw_pending_exception_object, align 8
  store ptr %outer_exception_field3, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 %outer_exception_field4, ptr @__thaw_pending_exception_value_tag, align 4
  store double %outer_exception_field5, ptr @__thaw_pending_exception_f64, align 8
  store i64 %outer_exception_field6, ptr @__thaw_pending_exception_i64, align 4
  store i1 %outer_exception_field7, ptr @__thaw_pending_exception_bool, align 1
  ret ptr %run_isolated_host_callback

restore_parked_native_throw:                      ; preds = %materialize_parked_native_throw
  %parked_native_throw_snapshot_field = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 0
  %parked_native_throw_field = load i64, ptr %parked_native_throw_snapshot_field, align 4
  %parked_native_throw_snapshot_field8 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 1
  %parked_native_throw_field9 = load i64, ptr %parked_native_throw_snapshot_field8, align 4
  %parked_native_throw_snapshot_field10 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 2
  %parked_native_throw_field11 = load i64, ptr %parked_native_throw_snapshot_field10, align 4
  %parked_native_throw_snapshot_field12 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 3
  %parked_native_throw_field13 = load i64, ptr %parked_native_throw_snapshot_field12, align 4
  %parked_native_throw_snapshot_field14 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 4
  %parked_native_throw_field15 = load i64, ptr %parked_native_throw_snapshot_field14, align 4
  %parked_native_throw_snapshot_field16 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 5
  %parked_native_throw_field17 = load i64, ptr %parked_native_throw_snapshot_field16, align 4
  %parked_native_throw_snapshot_field18 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 6
  %parked_native_throw_field19 = load i64, ptr %parked_native_throw_snapshot_field18, align 4
  %parked_native_throw_snapshot_field20 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 7
  %parked_native_throw_field21 = load i64, ptr %parked_native_throw_snapshot_field20, align 4
  %parked_native_throw_snapshot_field22 = getelementptr [9 x i64], ptr %parked_native_throw_snapshot_cell, i32 0, i32 8
  %parked_native_throw_field23 = load i64, ptr %parked_native_throw_snapshot_field22, align 4
  %parked_native_throw_number = bitcast i64 %parked_native_throw_field9 to double
  %parked_native_throw_bool = trunc i64 %parked_native_throw_field13 to i1
  %parked_native_throw_object = inttoptr i64 %parked_native_throw_field15 to ptr
  %parked_native_throw_native = inttoptr i64 %parked_native_throw_field17 to ptr
  %parked_native_throw_pending = inttoptr i64 %parked_native_throw_field19 to ptr
  %parked_native_throw_native_text = inttoptr i64 %parked_native_throw_field21 to ptr
  %parked_native_throw_aggregate = inttoptr i64 %parked_native_throw_field23 to ptr
  store i64 %parked_native_throw_field, ptr @__thaw_pending_exception_value_tag, align 4
  store double %parked_native_throw_number, ptr @__thaw_pending_exception_f64, align 8
  store i64 %parked_native_throw_field11, ptr @__thaw_pending_exception_i64, align 4
  store i1 %parked_native_throw_bool, ptr @__thaw_pending_exception_bool, align 1
  store ptr %parked_native_throw_object, ptr @__thaw_pending_exception_object, align 8
  store ptr %parked_native_throw_native, ptr @__thaw_pending_exception_native, align 8
  store ptr %parked_native_throw_pending, ptr @__thaw_pending_exception, align 8
  store ptr %parked_native_throw_native_text, ptr @__thaw_pending_exception_native_text, align 8
  store ptr %parked_native_throw_aggregate, ptr @__thaw_pending_exception_aggregate_errors, align 8
  br label %parked_native_throw_checked

materialize_parked_native_throw:                  ; preds = %check_parked_native_throw_live
  %parked_native_throw_materialized = call i8 @thaw_exception_native_snapshot(i64 %parked_native_throw_token, ptr %parked_native_throw_snapshot_cell)
  %parked_native_throw_snapshot_ok = icmp ne i8 %parked_native_throw_materialized, 0
  br i1 %parked_native_throw_snapshot_ok, label %restore_parked_native_throw, label %parked_native_throw_checked

parked_native_throw_checked:                      ; preds = %restore_parked_native_throw, %materialize_parked_native_throw, %check_parked_native_throw_live, %callback_raised
  %callback_restored_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %callback_thrown_tag = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %callback_thrown_f64 = load double, ptr @__thaw_pending_exception_f64, align 8
  %callback_thrown_i64 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %callback_thrown_bool = load i1, ptr @__thaw_pending_exception_bool, align 1
  %callback_thrown_bool_byte = zext i1 %callback_thrown_bool to i8
  %callback_thrown_object = load ptr, ptr @__thaw_pending_exception_object, align 8
  %callback_thrown_native = load ptr, ptr @__thaw_pending_exception_native, align 8
  %callback_thrown_native_text = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %callback_thrown_aggregate_errors = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %frame_isolated_callback_exception = call ptr @thaw_json_callback_error_tuple(ptr %callback_restored_exception, i64 %callback_thrown_tag, double %callback_thrown_f64, i64 %callback_thrown_i64, i8 %callback_thrown_bool_byte, ptr %callback_thrown_object, ptr %callback_thrown_native, ptr %callback_thrown_native_text, ptr %callback_thrown_aggregate_errors)
  store ptr %outer_exception_field, ptr @__thaw_pending_exception, align 8
  store ptr %outer_exception_field1, ptr @__thaw_pending_exception_native_text, align 8
  store ptr %outer_exception_field2, ptr @__thaw_pending_exception_object, align 8
  store ptr %outer_exception_field3, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 %outer_exception_field4, ptr @__thaw_pending_exception_value_tag, align 4
  store double %outer_exception_field5, ptr @__thaw_pending_exception_f64, align 8
  store i64 %outer_exception_field6, ptr @__thaw_pending_exception_i64, align 4
  store i1 %outer_exception_field7, ptr @__thaw_pending_exception_bool, align 1
  ret ptr %frame_isolated_callback_exception

check_parked_native_throw_live:                   ; preds = %callback_raised
  %parked_native_throw_live = call i8 @thaw_exception_token_is_live(i64 %parked_native_throw_token)
  %parked_native_throw_is_live = icmp ne i8 %parked_native_throw_live, 0
  br i1 %parked_native_throw_is_live, label %materialize_parked_native_throw, label %parked_native_throw_checked
}

define internal ptr @__thaw_lambda_9(ptr %0, double %1) #1 {
entry:
  %__thaw_json_wrap_value_4_cell1 = call ptr @thaw_arena_alloc(i64 ptrtoint (ptr getelementptr (double, ptr null, i32 1) to i64), i64 8)
  %__thaw_json_wrap_value_4_cell = alloca double, align 8
  store double %1, ptr %__thaw_json_wrap_value_4_cell, align 8
  %captured_stack_value = load double, ptr %__thaw_json_wrap_value_4_cell, align 8
  store double %captured_stack_value, ptr %__thaw_json_wrap_value_4_cell1, align 8
  %closure_alloc = call ptr @thaw_arena_alloc(i64 24, i64 8)
  store ptr @__thaw_lambda_10, ptr %closure_alloc, align 8
  %closure_this_entry = getelementptr inbounds i8, ptr %closure_alloc, i64 8
  store ptr @__thaw_lambda_10__thaw_this_adapter, ptr %closure_this_entry, align 8
  %capture_slot = getelementptr inbounds i8, ptr %closure_alloc, i64 16
  store ptr %__thaw_json_wrap_value_4_cell1, ptr %capture_slot, align 8
  %callable_is_undefined = icmp eq ptr %closure_alloc, null
  br i1 %callable_is_undefined, label %undefined_callable, label %present_callable

undefined_callable:                               ; preds = %entry
  %synthetic_error_frame = call ptr @thaw_set_synthetic_error(ptr @synthetic_error_name, ptr @synthetic_error_message.6)
  ret ptr null

present_callable:                                 ; preds = %entry
  %closure_code = load ptr, ptr %closure_alloc, align 8
  %closure_call = call ptr %closure_code(ptr %closure_alloc)
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

call_ok:                                          ; preds = %present_callable
  ret ptr %closure_call

propagate_exception:                              ; preds = %present_callable
  ret ptr null
}

define internal ptr @__thaw_lambda_9__thaw_this_adapter(ptr %0, { i8, i64 } %1, double %2) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_9(ptr %0, double %2)
  ret ptr %invoke_ignoring_this
}

define internal ptr @__thaw_lambda_10(ptr %0) #1 {
entry:
  %__thaw_json_wrap_obj_5_cell = alloca ptr, align 8
  %stack_probe_cell = alloca i8, align 1
  %captured = getelementptr inbounds i8, ptr %0, i64 16
  %capture_cell = load ptr, ptr %captured, align 8
  %stack_pointer = ptrtoint ptr %stack_probe_cell to i64
  %stack_limit = load i64, ptr @__thaw_stack_limit, align 4
  %stack_overflow = icmp ult i64 %stack_pointer, %stack_limit
  br i1 %stack_overflow, label %stack_overflow1, label %stack_ok, !prof !0

stack_overflow1:                                  ; preds = %entry
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store i64 4, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  store ptr @strlit.4, ptr @__thaw_pending_exception_native_text, align 8
  store ptr @strlit.4, ptr @__thaw_pending_exception, align 8
  ret ptr null

stack_ok:                                         ; preds = %entry
  %dictionary = call ptr @thaw_json_object_new()
  store ptr %dictionary, ptr %__thaw_json_wrap_obj_5_cell, align 8
  %__thaw_json_wrap_obj_5 = load ptr, ptr %__thaw_json_wrap_obj_5_cell, align 8
  %__thaw_json_wrap_value_4 = load double, ptr %capture_cell, align 8
  call void @thaw_json_object_set_number(ptr %__thaw_json_wrap_obj_5, ptr @strlit.5, double %__thaw_json_wrap_value_4)
  %json_host_error = call ptr @thaw_json_take_host_error()
  %json_host_has_error = icmp ne ptr %json_host_error, null
  br i1 %json_host_has_error, label %json_host_failed, label %json_host_succeeded

json_host_failed:                                 ; preds = %stack_ok
  store ptr %json_host_error, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception = icmp ne ptr %pending_exception, null
  br i1 %has_pending_exception, label %propagate_exception, label %call_ok

json_host_succeeded:                              ; preds = %call_ok, %stack_ok
  %__thaw_json_wrap_obj_52 = load ptr, ptr %__thaw_json_wrap_obj_5_cell, align 8
  %json_get = call ptr @thaw_json_take(ptr %__thaw_json_wrap_obj_52, ptr @strlit.5)
  %json_host_error3 = call ptr @thaw_json_take_host_error()
  %json_host_has_error6 = icmp ne ptr %json_host_error3, null
  br i1 %json_host_has_error6, label %json_host_failed4, label %json_host_succeeded5

call_ok:                                          ; preds = %json_host_failed
  br label %json_host_succeeded

propagate_exception:                              ; preds = %json_host_failed
  ret ptr null

json_host_failed4:                                ; preds = %json_host_succeeded
  call void @thaw_json_destroy(ptr %json_get)
  store ptr %json_host_error3, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr %json_host_error3, ptr @__thaw_pending_exception_native_text, align 8
  %pending_exception7 = load ptr, ptr @__thaw_pending_exception, align 8
  %has_pending_exception8 = icmp ne ptr %pending_exception7, null
  br i1 %has_pending_exception8, label %propagate_exception10, label %call_ok9

json_host_succeeded5:                             ; preds = %call_ok9, %json_host_succeeded
  ret ptr %json_get

call_ok9:                                         ; preds = %json_host_failed4
  br label %json_host_succeeded5

propagate_exception10:                            ; preds = %json_host_failed4
  ret ptr null
}

define internal ptr @__thaw_lambda_10__thaw_this_adapter(ptr %0, { i8, i64 } %1) #1 {
entry:
  %invoke_ignoring_this = call ptr @__thaw_lambda_10(ptr %0)
  ret ptr %invoke_ignoring_this
}

define internal void @__thaw_promise_resolve_11(ptr %0, { i1, ptr } %1) #1 {
entry:
  %resolver_state_capture = getelementptr inbounds i8, ptr %0, i64 16
  %resolver_state = load ptr, ptr %resolver_state_capture, align 8
  %resolver_called_slot = getelementptr inbounds i8, ptr %resolver_state, i64 8
  %resolver_called = load i8, ptr %resolver_called_slot, align 1
  %resolver_uncalled = icmp eq i8 %resolver_called, 0
  br i1 %resolver_uncalled, label %resolver_first_call, label %resolver_complete

resolver_first_call:                              ; preds = %entry
  store i8 1, ptr %resolver_called_slot, align 1
  %promise = load ptr, ptr %resolver_state, align 8
  %resolve_kind = extractvalue { i1, ptr } %1, 0
  br i1 %resolve_kind, label %resolve_adopt, label %resolve_plain

resolver_complete:                                ; preds = %resolve_done, %entry
  ret void

resolve_adopt:                                    ; preds = %resolver_first_call
  %resolve_promise = extractvalue { i1, ptr } %1, 1
  %adopt_resolve_value = call i8 @thaw_promise_adopt(ptr %promise, ptr %resolve_promise)
  br label %resolve_done

resolve_plain:                                    ; preds = %resolver_first_call
  %settle_resolve_value = call i8 @thaw_promise_resolve(ptr %promise, ptr null)
  br label %resolve_done

resolve_done:                                     ; preds = %resolve_plain, %resolve_adopt
  br label %resolver_complete
}

define internal void @__thaw_promise_reject_12(ptr %0, ptr %1) #1 {
entry:
  %resolver_state_capture = getelementptr inbounds i8, ptr %0, i64 16
  %resolver_state = load ptr, ptr %resolver_state_capture, align 8
  %resolver_called_slot = getelementptr inbounds i8, ptr %resolver_state, i64 8
  %resolver_called = load i8, ptr %resolver_called_slot, align 1
  %resolver_uncalled = icmp eq i8 %resolver_called, 0
  br i1 %resolver_uncalled, label %resolver_first_call, label %resolver_complete

resolver_first_call:                              ; preds = %entry
  store i8 1, ptr %resolver_called_slot, align 1
  %promise = load ptr, ptr %resolver_state, align 8
  store ptr null, ptr @__thaw_pending_exception_native, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 4, ptr @__thaw_pending_exception_value_tag, align 4
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store ptr %1, ptr @__thaw_pending_exception_native_text, align 8
  %promise_exception_value = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %promise_exception_value1 = load double, ptr @__thaw_pending_exception_f64, align 8
  %promise_exception_value2 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %promise_exception_value3 = load i1, ptr @__thaw_pending_exception_bool, align 1
  %promise_exception_value4 = load ptr, ptr @__thaw_pending_exception_object, align 8
  %pending_native_text_provenance = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %pending_aggregate_errors = load ptr, ptr @__thaw_pending_exception_aggregate_errors, align 8
  %pending_native_exception_descriptor = load ptr, ptr @__thaw_pending_exception_native, align 8
  %pending_error_bits = ptrtoint ptr %1 to i64
  %pending_native_text_bits = ptrtoint ptr %pending_native_text_provenance to i64
  %aggregate_text_matches_error = icmp eq i64 %pending_error_bits, %pending_native_text_bits
  %aggregate_has_trusted_text = icmp ne ptr %pending_native_text_provenance, null
  %aggregate_trusted_pair = and i1 %aggregate_text_matches_error, %aggregate_has_trusted_text
  %paired_aggregate_errors = select i1 %aggregate_trusted_pair, ptr %pending_aggregate_errors, ptr null
  %settle_promise = call i8 @thaw_promise_reject_typed_with_native_provenance(ptr %promise, ptr %1, i64 %promise_exception_value, double %promise_exception_value1, i64 %promise_exception_value2, i1 %promise_exception_value3, ptr %promise_exception_value4, ptr %pending_native_text_provenance, ptr %paired_aggregate_errors, ptr %pending_native_exception_descriptor)
  br label %resolver_complete

resolver_complete:                                ; preds = %resolver_first_call, %entry
  ret void
}

define internal void @resolve_closure__thaw_this_adapter(ptr %0, { i8, i64 } %1, { i1, ptr } %2) #1 {
entry:
  call void @__thaw_promise_resolve_11(ptr %0, { i1, ptr } %2)
  ret void
}

define internal void @reject_closure__thaw_this_adapter(ptr %0, { i8, i64 } %1, ptr %2) #1 {
entry:
  call void @__thaw_promise_reject_12(ptr %0, ptr %2)
  ret void
}

define i32 @main() #1 {
entry:
  %terminal_failure = alloca i1, align 1
  %shutdown_error_seen = alloca i1, align 1
  call void @thaw_register_fn_table(ptr @thaw_stack_table, i64 4, ptr @__start_thaw_user_fns, ptr @__stop_thaw_user_fns, ptr @stack_file)
  call void @thaw_json_register_host_operations(ptr @thaw_js_retain_handle, ptr @thaw_js_release_handle, ptr @thaw_js_get_property_json_key_result, ptr @thaw_js_host_query_result, ptr @thaw_js_host_date_set_result, ptr @thaw_js_set_property_graph_result, ptr @thaw_js_property_predicate_json_key_result, ptr @thaw_js_host_enumerate_result)
  call void @thaw_json_register_error_factory(ptr @thaw_js_error_from_tagged_handle, ptr @thaw_js_lookup_native_object, ptr @thaw_js_intern_native_object)
  %js_engine_id = call i8 @thaw_js_engine_id()
  call void @thaw_json_set_js_engine(i8 %js_engine_id)
  %quickjs_event_loop_status = alloca i32, align 4
  store i32 0, ptr %quickjs_event_loop_status, align 4
  call void @thaw_arena_enable_tracing()
  %stack_guard_limit = call i64 @thaw_stack_guard_limit()
  store i64 %stack_guard_limit, ptr @__thaw_stack_limit, align 4
  call void @thaw_stack_guard_register(ptr @__thaw_stack_limit)
  call void @thaw_promise_set_unhandled_reporter_text_result(ptr @thaw_js_emit_unhandled_rejection_result)
  call void @thaw_promise_set_rejection_handled_reporter_result(ptr @thaw_js_emit_rejection_handled_result)
  %run_quickjs_cli = call i32 @thaw_js_run_cli()
  store i32 %run_quickjs_cli, ptr %quickjs_event_loop_status, align 4
  %quickjs_cli_handled = icmp sge i32 %run_quickjs_cli, 0
  br i1 %quickjs_cli_handled, label %entry_cleanup, label %ordinary_entry

entry_cleanup:                                    ; preds = %async_main_completion_released, %entry
  store i1 false, ptr %terminal_failure, align 1
  br label %terminal_ordinary_notifications

ordinary_entry:                                   ; preds = %entry
  store i32 0, ptr %quickjs_event_loop_status, align 4
  %call_thaw_user_main = call ptr @thaw_user_main()
  %run_async_main = call ptr @thaw_runtime_run_until_resolved(ptr %call_thaw_user_main)
  call void @thaw_js_run_until_native_resolved(ptr %call_thaw_user_main)
  call void @thaw_runtime_report_unhandled()
  %async_main_state = call i8 @thaw_promise_state(ptr %call_thaw_user_main)
  %async_main_settled = icmp ne i8 %async_main_state, 0
  br i1 %async_main_settled, label %destroy_settled_async_main, label %leak_pending_async_main

destroy_settled_async_main:                       ; preds = %ordinary_entry
  call void @thaw_promise_destroy(ptr %call_thaw_user_main)
  br label %async_main_completion_released

leak_pending_async_main:                          ; preds = %ordinary_entry
  br label %async_main_completion_released

async_main_completion_released:                   ; preds = %leak_pending_async_main, %destroy_settled_async_main
  %drain_detached_promises = call i64 @thaw_runtime_drain_detached()
  %drain_native_microtasks = call i64 @thaw_runtime_run_until_idle()
  %run_quickjs_event_loop = call i32 @thaw_js_run_event_loop()
  store i32 %run_quickjs_event_loop, ptr %quickjs_event_loop_status, align 4
  br label %entry_cleanup

terminal_ordinary_notifications:                  ; preds = %terminal_wait_for_work, %entry_cleanup
  %process_pending_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %original_exception_native_text = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %original_exception_typed_field = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %original_exception_typed_field1 = load double, ptr @__thaw_pending_exception_f64, align 8
  %original_exception_typed_field2 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %original_exception_typed_field3 = load i1, ptr @__thaw_pending_exception_bool, align 1
  %original_exception_owned_report_text = call ptr @thaw_runtime_exception_report_text(ptr %process_pending_exception, ptr %original_exception_native_text, i64 %original_exception_typed_field, double %original_exception_typed_field1, i64 %original_exception_typed_field2, i1 %original_exception_typed_field3)
  store ptr null, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  %process_exception_failed = icmp ne ptr %process_pending_exception, null
  %emit_process_uncaught_exception = call { i64, ptr } @thaw_js_emit_uncaught_result(ptr %original_exception_owned_report_text)
  %process_exception_handled_value = extractvalue { i64, ptr } %emit_process_uncaught_exception, 0
  %process_exception_listener_error = extractvalue { i64, ptr } %emit_process_uncaught_exception, 1
  %process_listener_failed = icmp ne ptr %process_exception_listener_error, null
  %process_exception_handled = icmp ne i64 %process_exception_handled_value, 0
  %process_handler_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %listener_exception_native_text = load ptr, ptr @__thaw_pending_exception_native_text, align 8
  %listener_exception_typed_field = load i64, ptr @__thaw_pending_exception_value_tag, align 4
  %listener_exception_typed_field4 = load double, ptr @__thaw_pending_exception_f64, align 8
  %listener_exception_typed_field5 = load i64, ptr @__thaw_pending_exception_i64, align 4
  %listener_exception_typed_field6 = load i1, ptr @__thaw_pending_exception_bool, align 1
  %listener_exception_owned_report_text = call ptr @thaw_runtime_exception_report_text(ptr %process_handler_exception, ptr %listener_exception_native_text, i64 %listener_exception_typed_field, double %listener_exception_typed_field4, i64 %listener_exception_typed_field5, i1 %listener_exception_typed_field6)
  store ptr null, ptr @__thaw_pending_exception, align 8
  store ptr null, ptr @__thaw_pending_exception_native_text, align 8
  store ptr null, ptr @__thaw_pending_exception_object, align 8
  store ptr null, ptr @__thaw_pending_exception_aggregate_errors, align 8
  store i64 0, ptr @__thaw_pending_exception_value_tag, align 4
  store double 0.000000e+00, ptr @__thaw_pending_exception_f64, align 8
  store i64 0, ptr @__thaw_pending_exception_i64, align 4
  store i1 false, ptr @__thaw_pending_exception_bool, align 1
  %process_handler_failed = icmp ne ptr %process_handler_exception, null
  %process_exception_unhandled = xor i1 %process_exception_handled, true
  %process_original_exception_unhandled = and i1 %process_exception_failed, %process_exception_unhandled
  %process_exception_failed_unhandled = or i1 %process_original_exception_unhandled, %process_handler_failed
  %process_listener_exception_failed = or i1 %process_exception_failed_unhandled, %process_listener_failed
  %original_uncaught_exception = select i1 %process_original_exception_unhandled, ptr %original_exception_owned_report_text, ptr null
  call void @thaw_runtime_report_uncaught(ptr %original_uncaught_exception)
  call void @thaw_runtime_report_uncaught(ptr %listener_exception_owned_report_text)
  call void @thaw_runtime_report_uncaught(ptr %process_exception_listener_error)
  call void @thaw_cstring_destroy(ptr %process_exception_listener_error)
  call void @thaw_cstring_destroy(ptr %original_exception_owned_report_text)
  call void @thaw_cstring_destroy(ptr %listener_exception_owned_report_text)
  %process_pending_rejection = load ptr, ptr @__thaw_pending_rejection, align 8
  store ptr null, ptr @__thaw_pending_rejection, align 8
  %process_rejection_failed = icmp ne ptr %process_pending_rejection, null
  %emit_process_unhandled_rejection = call { i64, ptr } @thaw_js_emit_unhandled_rejection_result(ptr %process_pending_rejection)
  %process_rejection_handled_value = extractvalue { i64, ptr } %emit_process_unhandled_rejection, 0
  %process_rejection_listener_error = extractvalue { i64, ptr } %emit_process_unhandled_rejection, 1
  %process_rejection_listener_failed = icmp ne ptr %process_rejection_listener_error, null
  %process_rejection_handled = icmp ne i64 %process_rejection_handled_value, 0
  %process_rejection_unhandled = xor i1 %process_rejection_handled, true
  %process_rejection_failed_unhandled = and i1 %process_rejection_failed, %process_rejection_unhandled
  %process_rejection_handler_failed = or i1 %process_rejection_failed_unhandled, %process_rejection_listener_failed
  %reported_unhandled_rejection = select i1 %process_rejection_handled, ptr null, ptr %process_pending_rejection
  call void @thaw_runtime_report_uncaught(ptr %reported_unhandled_rejection)
  call void @thaw_runtime_report_uncaught(ptr %process_rejection_listener_error)
  call void @thaw_cstring_destroy(ptr %process_rejection_listener_error)
  call void @thaw_cstring_destroy(ptr %process_pending_rejection)
  %drain_stored_unhandled_rejections = call i8 @thaw_promise_drain_unhandled_text_result(ptr @thaw_js_emit_unhandled_rejection_result)
  %stored_rejection_failed = icmp ne i8 %drain_stored_unhandled_rejections, 0
  %take_checkpoint_rejection_failure = call i8 @thaw_promise_take_unhandled_failure()
  %checkpoint_rejection_failed = icmp ne i8 %take_checkpoint_rejection_failure, 0
  %direct_script_failed = or i1 %process_listener_exception_failed, %process_rejection_handler_failed
  %exit_script_failed = or i1 %direct_script_failed, %stored_rejection_failed
  %script_failed = or i1 %exit_script_failed, %checkpoint_rejection_failed
  %process_failed = or i1 %script_failed, false
  %previous_terminal_failure = load i1, ptr %terminal_failure, align 1
  %terminal_failure_latched = or i1 %previous_terminal_failure, %process_failed
  store i1 %terminal_failure_latched, ptr %terminal_failure, align 1
  br i1 %process_failed, label %terminal_fatal_exit, label %terminal_after_fatal_exit

terminal_begin_shutdown:                          ; preds = %terminal_after_fatal_exit
  %release_terminal_handles = call i64 @thaw_js_release_all_handles()
  %released_terminal_handles = icmp ne i64 %release_terminal_handles, 0
  br i1 %released_terminal_handles, label %terminal_wait_for_work, label %terminal_finished

terminal_wait_for_work:                           ; preds = %terminal_begin_shutdown, %terminal_after_fatal_exit
  call void @thaw_runtime_shutdown_wait()
  br label %terminal_ordinary_notifications

terminal_finished:                                ; preds = %terminal_begin_shutdown
  %shutdown_quickjs_context = call i8 @thaw_js_shutdown_context()
  %terminal_failure_status = load i1, ptr %terminal_failure, align 1
  %quickjs_event_loop_status9 = load i32, ptr %quickjs_event_loop_status, align 4
  %failure_exit_status = zext i1 %terminal_failure_status to i32
  %process_exit_status = select i1 %terminal_failure_status, i32 %failure_exit_status, i32 %quickjs_event_loop_status9
  ret i32 %process_exit_status

terminal_fatal_exit:                              ; preds = %terminal_ordinary_notifications
  call void @thaw_runtime_abort_pending_work()
  call void @thaw_js_fatal_exit()
  br label %terminal_after_fatal_exit

terminal_after_fatal_exit:                        ; preds = %terminal_fatal_exit, %terminal_ordinary_notifications
  %terminal_native_before_js = call i64 @thaw_runtime_run_until_idle()
  %terminal_native_before_progress = icmp ne i64 %terminal_native_before_js, 0
  %terminal_quickjs_jobs = call i32 @thaw_js_run_event_loop()
  %terminal_quickjs_failed = icmp ne i32 %terminal_quickjs_jobs, 0
  %terminal_previous_quickjs_status = load i32, ptr %quickjs_event_loop_status, align 4
  %terminal_quickjs_status = select i1 %terminal_quickjs_failed, i32 %terminal_quickjs_jobs, i32 %terminal_previous_quickjs_status
  store i32 %terminal_quickjs_status, ptr %quickjs_event_loop_status, align 4
  %terminal_quickjs_pending_query = call i8 @thaw_js_terminal_work_pending()
  %terminal_quickjs_work_pending = icmp ne i8 %terminal_quickjs_pending_query, 0
  %terminal_quickjs_more_work = or i1 %terminal_native_before_progress, %terminal_quickjs_work_pending
  %terminal_native_after_js = call i64 @thaw_runtime_run_until_idle()
  %terminal_native_after_progress = icmp ne i64 %terminal_native_after_js, 0
  %terminal_native_progress = or i1 %terminal_quickjs_more_work, %terminal_native_after_progress
  %terminal_promise_notifications = call i64 @thaw_promise_take_report_activity()
  %terminal_promise_reported = icmp ne i64 %terminal_promise_notifications, 0
  %terminal_reporting_progress = or i1 %terminal_native_progress, %terminal_promise_reported
  %terminal_new_exception = load ptr, ptr @__thaw_pending_exception, align 8
  %terminal_pending_notification = icmp ne ptr %terminal_new_exception, null
  %terminal_more_notifications = or i1 %terminal_reporting_progress, %terminal_pending_notification
  %terminal_new_rejection = load ptr, ptr @__thaw_pending_rejection, align 8
  %terminal_pending_notification7 = icmp ne ptr %terminal_new_rejection, null
  %terminal_more_notifications8 = or i1 %terminal_more_notifications, %terminal_pending_notification7
  %terminal_pending_native_async = call i8 @thaw_runtime_async_work_pending()
  %terminal_native_async_pending = icmp ne i8 %terminal_pending_native_async, 0
  %terminal_native_async_progress = or i1 %terminal_more_notifications8, %terminal_native_async_pending
  br i1 %terminal_native_async_progress, label %terminal_wait_for_work, label %terminal_begin_shutdown
}

declare void @thaw_arena_enable_tracing()

declare void @thaw_arena_register_root(ptr)

declare i64 @thaw_stack_guard_limit()

declare void @thaw_stack_guard_register(ptr)

attributes #0 = { nocallback nocreateundeforpoison nofree nosync nounwind speculatable willreturn memory(none) }
attributes #1 = { "frame-pointer"="all" }
attributes #2 = { noinline "frame-pointer"="all" }

!0 = !{!"branch_weights", i32 1, i32 1048576}
