//! Every fixed global name the compiler passes by name to `getDynamicValue`, and the names thaw-llvm
//! dispatches itself. Emitters use these constants. The three registry lists below are checked by
//! `dynamic_helper_contract_tests` in thaw-js. Program- or package-derived names (`dynamic_symbol`, class and
//! namespace names, per-binding `__thaw_method_callable_*`) are variables, not fixed names: they are NOT listed.

pub const JSON_STRINGIFY_LIVE_REPLACER: &str = "__thaw_json_stringify_live_replacer";
pub const JSON_STRINGIFY_LIVE_DYNAMIC_REPLACER: &str =
    "__thaw_json_stringify_live_dynamic_replacer";
pub const JSON_STRINGIFY_NATIVE_ACCESSORS: &str = "__thaw_json_stringify_native_accessors";
pub const ACCESSOR_DESCRIPTOR: &str = "__thaw_accessor_descriptor";
pub const DATA_DESCRIPTOR: &str = "__thaw_data_descriptor";
pub const NUMBER_CONSTRUCTOR: &str = "__thaw_number_constructor";
pub const DATE_FROM_LIVE_VALUE: &str = "__thaw_date_from_live_value";
pub const ARRAY_WITH_NATIVE_VIEW: &str = "__thaw_array_with_native_view";
pub const CALL_SELECTED_DYNAMIC_METHOD_HANDLE: &str = "__thaw_call_selected_dynamic_method_handle";
pub const COLLECTION_WITH_NATIVE_VIEW: &str = "__thaw_collection_with_native_view";
pub const DEFER_THENABLE: &str = "__thaw_defer_thenable";
pub const DYNAMIC_ADD: &str = "__thaw_dynamic_add";
pub const DYNAMIC_BINARY: &str = "__thaw_dynamic_binary";
pub const ERROR_CONSTRUCTOR_NAME: &str = "__thaw_error_constructor_name";
pub const INSTANCEOF_DYNAMIC_VALUE: &str = "__thaw_instanceof_dynamic_value";
pub const INSTANCEOF_DYNAMIC_VALUE_BY_NAME: &str = "__thaw_instanceof_dynamic_value_by_name";
pub const IS_TRUTHY_DYNAMIC_VALUE: &str = "__thaw_is_truthy_dynamic_value";
pub const ITERATOR_FROM: &str = "__thaw_iterator_from";
pub const JSON_HOST_FROM_DYNAMIC: &str = "__thaw_json_host_from_dynamic";
pub const JSON_HOST_INSTANCEOF: &str = "__thaw_json_host_instanceof";
pub const JSON_STRINGIFY_DIRECT: &str = "__thaw_json_stringify_direct";
pub const JSON_UNDEFINED: &str = "__thaw_json_undefined";
pub const OBJECT_FROM_OPERATIONS: &str = "__thaw_object_from_operations";
pub const OBJECT_WITH_NATIVE_GETTERS: &str = "__thaw_object_with_native_getters";
pub const STORAGE_VALUE: &str = "__thaw_storage_value";
pub const STRICT_EQUAL_DYNAMIC: &str = "__thaw_strict_equal_dynamic";
pub const TO_ITERATOR_EXACT: &str = "__thaw_to_iterator_exact";
pub const TYPEOF_DYNAMIC_VALUE: &str = "__thaw_typeof_dynamic_value";
pub const UPDATE_DYNAMIC_VALUE: &str = "__thaw_update_dynamic_value";
pub const NATIVE_LOOSE_EQUAL: &str = "__thaw_native_loose_equal";
pub const UNARY_PLUS: &str = "__thaw_unary_plus";
pub const UNDEFINED: &str = "undefined";

/// Fixed JS globals looked up by name (`getDynamicValue`): each must be defined in the thaw-js default realm.
pub const GLOBAL_BIGINT: &str = "BigInt";
pub const GLOBAL_MATH: &str = "Math";
pub const GLOBAL_REGEXP: &str = "RegExp";

/// Aliases of other JS globals used as plain literal values (not `getDynamicValue` arguments).
pub const GLOBAL_OBJECT: &str = "Object";
pub const GLOBAL_JSON: &str = "JSON";
pub const GLOBAL_REFLECT: &str = "Reflect";
pub const GLOBAL_PROMISE: &str = "Promise";
pub const GLOBAL_PROXY: &str = "Proxy";
pub const GLOBAL_ITERATOR: &str = "Iterator";
pub const GLOBAL_ARRAY: &str = "Array";
pub const GLOBAL_FUNCTION: &str = "Function";
pub const GLOBAL_THIS: &str = "globalThis";

pub const COMPILER_EMITTED_JS_HELPERS: &[&str] = &[
    ACCESSOR_DESCRIPTOR,
    ARRAY_WITH_NATIVE_VIEW,
    COLLECTION_WITH_NATIVE_VIEW,
    DATA_DESCRIPTOR,
    DATE_FROM_LIVE_VALUE,
    DYNAMIC_ADD,
    DYNAMIC_BINARY,
    INSTANCEOF_DYNAMIC_VALUE,
    INSTANCEOF_DYNAMIC_VALUE_BY_NAME,
    IS_TRUTHY_DYNAMIC_VALUE,
    ITERATOR_FROM,
    JSON_HOST_INSTANCEOF,
    JSON_STRINGIFY_DIRECT,
    JSON_STRINGIFY_LIVE_REPLACER,
    JSON_STRINGIFY_LIVE_DYNAMIC_REPLACER,
    JSON_STRINGIFY_NATIVE_ACCESSORS,
    NUMBER_CONSTRUCTOR,
    OBJECT_FROM_OPERATIONS,
    OBJECT_WITH_NATIVE_GETTERS,
    STORAGE_VALUE,
    TYPEOF_DYNAMIC_VALUE,
    UPDATE_DYNAMIC_VALUE,
    STRICT_EQUAL_DYNAMIC,
    NATIVE_LOOSE_EQUAL,
    UNARY_PLUS,
];

pub const JS_GLOBALS: &[&str] = &[GLOBAL_BIGINT, GLOBAL_MATH, GLOBAL_REGEXP];

/// Names the compiler passes that are provided natively (thaw-llvm codegen or the thaw-js host), not by a
/// realm global. `(name, provider file relative to the repo root)`; the test checks the name is absent from the
/// realm and the provider file mentions it.
pub const HOST_PROVIDED_HELPERS: &[(&str, &str)] = &[
    // thaw-llvm dispatches this intrinsic by name (Var call), not a realm global
    (
        CALL_SELECTED_DYNAMIC_METHOD_HANDLE,
        "crates/thaw-llvm/src/hir_codegen/invocations/dynamic_calls.rs",
    ),
    // thaw-llvm intrinsic (Var call inside getDynamicValue argument)
    (
        ERROR_CONSTRUCTOR_NAME,
        "crates/thaw-llvm/src/hir_codegen/invocations/calls.rs",
    ),
    // thaw-llvm intrinsic (Var call), compiled to a host JSON copy
    (
        JSON_HOST_FROM_DYNAMIC,
        "crates/thaw-llvm/src/hir_codegen/invocations/dynamic_calls.rs",
    ),
    // thaw-llvm intrinsic (Var call) producing the Json undefined
    (
        JSON_UNDEFINED,
        "crates/thaw-llvm/src/hir_codegen/invocations/json_calls.rs",
    ),
    // thaw-llvm intrinsic (Var call), compiled inline
    (
        TO_ITERATOR_EXACT,
        "crates/thaw-llvm/src/hir_codegen/invocations/dynamic_calls.rs",
    ),
    // thaw_tjs_get_global special-cases the literal name `undefined` (handle for undefined)
    (UNDEFINED, "crates/thaw-js/src/host.rs"),
];
