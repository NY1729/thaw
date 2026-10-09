#[derive(Clone)]
struct NarrowingSnapshot {
    optional: HashMap<Symbol, HirType>,
    nullable: HashMap<Symbol, HirType>,
    nullish: HashMap<Symbol, HirType>,
    partial_nullable: HashMap<Symbol, HirType>,
    write_versions: HashMap<Symbol, u64>,
}

/// Lowers one function body while retaining its typed lexical scope.
struct FnLowerer<'a> {
    scope: HashMap<Symbol, HirType>,
    immutable_bindings: HashSet<Symbol>,
    /// Declared (annotated) function/method parameters whose type may be `undefined`/`null`:
    /// an un-narrowed property read on one is a compile error, as in `tsc`.
    strict_nullable_bindings: HashSet<Symbol>,
    static_string_bindings: HashMap<Symbol, String>,
    narrowings: HashMap<Symbol, HirType>,
    nullable_narrowings: HashMap<Symbol, HirType>,
    nullish_narrowings: HashMap<Symbol, HirType>,
    partial_nullable_narrowings: HashMap<Symbol, HirType>,
    narrowing_write_versions: HashMap<Symbol, u64>,
    last_if_condition_narrowing: Option<(Symbol, HirType, bool, u8)>,
    json_narrowings: HashMap<Symbol, HirType>,
    exception_object_narrowings: HashMap<Symbol, HirType>,
    union_narrowings: HashMap<Symbol, (Vec<usize>, Vec<HirType>)>,
    union_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    array_element_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    nested_array_discriminants: HashMap<Symbol, NestedArrayDiscriminants>,
    object_array_property_discriminants: HashMap<Symbol, ObjectArrayPropertyDiscriminants>,
    object_function_property_discriminants: HashMap<Symbol, ObjectFunctionPropertyDiscriminants>,
    destructured_union_correlations: HashMap<Symbol, DestructuredUnionCorrelation>,
    /// `for...of` over a `Json` value is lowered twice (array loop / live-iterable protocol);
    /// `Some(true)` selects the live-iterable lowering, `Some(false)` the array loop.
    for_of_json_mode: Option<bool>,
    /// The right-hand side already lowered by the `for...of` dispatch.
    for_of_prelowered: Option<HirExpr>,
    destructuring_default_types: HashMap<Symbol, HirType>,
    function_value_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    function_value_array_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    function_value_nested_array_discriminants: HashMap<Symbol, NestedArrayDiscriminants>,
    function_value_object_array_property_discriminants:
        HashMap<Symbol, ObjectArrayPropertyDiscriminants>,
    function_value_object_function_property_discriminants:
        HashMap<Symbol, ObjectFunctionPropertyDiscriminants>,
    bindings: HashMap<Symbol, Vec<Symbol>>,
    used_hir_bindings: HashSet<Symbol>,
    sparse_arrays: HashSet<Symbol>,
    /// `__thaw_native_arg_N` temporaries bound to a fresh object literal (not a native owner).
    fresh_object_bindings: HashSet<Symbol>,
    conservative_sparse_arrays: HashSet<Symbol>,
    sparse_array_functions: HashSet<Symbol>,
    next_binding: usize,
    signatures: &'a HashMap<Symbol, FnSignature>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'a>,
    enum_values: &'a EnumValues,
    enum_reverse_values: &'a EnumReverseValues,
    ret_type: HirType,
    call_constraints: Option<&'a RefCell<Vec<CallConstraint>>>,
    generic_call_returns: HashMap<Symbol, HirType>,
    generic_arrows: HashMap<Symbol, swc_ecma_ast::ArrowExpr>,
    generic_non_arrow_names: HashSet<Symbol>,
    generic_non_arrow_receivers: HashMap<Symbol, HirType>,
    generic_non_arrow_receiver_templates: HashMap<Symbol, Box<TsType>>,
    generic_arrow_self_names: HashMap<Symbol, Symbol>,
    generic_named_templates: HashMap<Symbol, Symbol>,
    native_method_values: HashMap<Symbol, NativeMethodValue>,
    native_class_aliases: HashMap<Symbol, HirType>,
    member_receiver_bindings: HashSet<Symbol>,
    awaited_bindings: HashSet<Symbol>,
    /// Names bound by a `catch (e)` clause. `typeof e` reports `"object"`
    /// for these (real JavaScript throws an `Error` object), even though
    /// the binding is stored as the tagged error *string* internally --
    /// see `statements/lowering.rs`'s catch handling.
    catch_bindings: HashSet<Symbol>,
    promise_catch_bindings: HashSet<Symbol>,
    promise_catch_parameter: Option<Symbol>,
    loop_depth: usize,
    labels: Vec<(Symbol, usize, bool)>,
    super_initializer: Option<(Symbol, HirType, Symbol)>,
    class_static_context: bool,
    class_context: Option<Symbol>,
    unbound_this_context: bool,
    non_arrow_receiver: Option<HirType>,
    /// A contextual type hint for the *next* call expression `lower_call`
    /// handles, consumed (taken, not just read) as its very first action
    /// so it can never leak into a nested/argument call's own inference --
    /// see `lower_expr_with_expected_type`'s doc comment for why this
    /// exists and how it stays scoped to exactly one call.
    expected_return_hint: Option<HirType>,
    /// Like `expected_return_hint`, but for the *body* of the next arrow
    /// function `lower_arrow` handles, taken the same one-shot way. An
    /// arrow with no return-type annotation otherwise defaults its own
    /// `ret_type` to `HirType::Dynamic`, which starves every `return`
    /// inside it of a hint -- so a dynamic method call in tail position
    /// (`return c.text(...)`, a real hono handler passed with no explicit
    /// `: JsValue` annotation) silently took the untyped JSON-snapshot
    /// path and handed hono back `{}` instead of the live `Response`.
    /// Set only when an arrow is lowered as a direct argument to a
    /// dynamic method call (`lower_dynamic_value_method_call`), the one
    /// place a callback's *unannotated* return is still known to need to
    /// stay a live `JsValue`.
    expected_arrow_return_hint: Option<HirType>,
    sparse_mapping_result: bool,
    generator_yields: Option<(Symbol, HirType, Symbol, HirType, Symbol, HirType)>,
    generator_finalizers: HashMap<Symbol, Vec<HirStmt>>,
    /// `new Proxy(target, handler)`'s own `target` variable, when it was
    /// a simple identifier declared `Object`/`Dictionary`-shaped in this
    /// same function: maps the original variable's own symbol to (the
    /// symbol of a live QuickJS handle holding an independent, retained
    /// copy of it, the variable's own original declared type). A read of
    /// the *original* identifier is redirected through this handle
    /// instead (see the `Expr::Ident` lowering arm) so a `set` trap's
    /// mutation -- which only ever reaches the QuickJS-side object, not
    /// this Thaw-side variable's own native storage -- is observable by
    /// reading the identifier directly, not just through the proxy
    /// itself. Scoped to the current function only, same precision as
    /// `scope` itself: a read after the identifier is passed to another
    /// function, or captured into a closure invoked later, isn't
    /// redirected (unaffected, not a regression -- that combination
    /// never worked before this either).
    proxy_target_live_handles: HashMap<Symbol, (Symbol, HirType)>,
}

#[derive(Clone)]
struct UnionNarrowingTarget {
    name: Symbol,
    matching: Vec<usize>,
    allowed: Vec<usize>,
    elements: Vec<HirType>,
}

type UnionTypeofNarrowing = (Vec<UnionNarrowingTarget>, bool, bool);

#[derive(Clone)]
struct CorrelatedUnionTarget {
    name: Symbol,
    elements: Vec<HirType>,
    source_members: Vec<Vec<usize>>,
}

#[derive(Clone)]
struct DestructuredUnionCorrelation {
    literals: Vec<Option<HirLit>>,
    targets: Vec<CorrelatedUnionTarget>,
}

struct CorrelatedDestructuredBinding {
    name: Symbol,
    ty: HirType,
    source_types: Vec<Vec<HirType>>,
}
