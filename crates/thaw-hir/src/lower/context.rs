#[derive(Clone)]
struct GenericBindCapture {
    name: Symbol,
    ty: HirType,
    source_span: Option<swc_common::Span>,
}

#[derive(Clone)]
struct DeferredGenericBind {
    target: GenericCallableTemplate,
    this_capture: GenericBindCapture,
    leading_captures: Vec<GenericBindCapture>,
}

#[derive(Clone)]
struct NarrowingSnapshot {
    optional: HashMap<Symbol, HirType>,
    nullable: HashMap<Symbol, HirType>,
    nullish: HashMap<Symbol, HirType>,
    partial_nullable: HashMap<Symbol, HirType>,
    write_versions: HashMap<Symbol, u64>,
}

/// Lowers one function body while retaining its typed lexical scope.
#[derive(Clone)]
struct FnLowerer<'a> {
    scope: HashMap<Symbol, HirType>,
    source_scope: HashMap<Symbol, HirType>,
    source_ret_type: Option<HirType>,
    immutable_bindings: HashSet<Symbol>,
    /// `var` names already declared in the current function, mapped to the
    /// HIR binding they own; a redeclaration reuses that binding.
    function_var_names: HashMap<String, Symbol>,
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
    for_of_native_iterator: bool,
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
    /// Set by the `any` declaration/assignment sites: the next native array converted to
    /// `Json` is a live view that aliases the array instead of a copy.
    alias_native_arrays: bool,
    /// Class layouts whose method results are being projected live right now; a method
    /// handing back its own (or an enclosing) class is not projected again.
    projecting_class_methods: Vec<HirType>,
    /// One-shot: parameter type for the next arrow passed to a live dynamic method call, instead of `JsValue`.
    dynamic_callback_parameter: Option<HirType>,
    /// Set while lowering the general `===` under a caught-Error identity check (no recursion).
    text_identity_pending: bool,
    /// While building a deferred projector factory, its generated method
    /// callbacks must not recursively register another projector factory.
    lowering_native_projection_factory: bool,
    conservative_sparse_arrays: HashSet<Symbol>,
    sparse_array_functions: HashSet<Symbol>,
    next_binding: usize,
    signatures: &'a HashMap<Symbol, FnSignature>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'a>,
    enum_values: &'a EnumValues,
    enum_reverse_values: &'a EnumReverseValues,
    ret_type: HirType,
    /// The function being lowered is `async` (its `ret_type` is the unwrapped result).
    async_return: bool,
    call_constraints: Option<&'a RefCell<Vec<CallConstraint>>>,
    generic_call_returns: HashMap<Symbol, HirType>,
    generic_arrows: HashMap<Symbol, swc_ecma_ast::ArrowExpr>,
    generic_non_arrow_names: HashSet<Symbol>,
    generic_non_arrow_receivers: HashMap<Symbol, HirType>,
    generic_non_arrow_receiver_templates: HashMap<Symbol, Box<TsType>>,
    generic_arrow_self_names: HashMap<Symbol, Symbol>,
    generic_named_templates: HashMap<Symbol, Symbol>,
    deferred_generic_binds: HashMap<Symbol, DeferredGenericBind>,
    native_method_values: HashMap<Symbol, NativeMethodValue>,
    native_class_aliases: HashMap<Symbol, HirType>,
    /// Set when a class instance was converted to a distinct interface view object.
    class_method_view_made: bool,
    /// Nesting of dynamic accessor projections (guards against recursive class types).
    accessor_projection_depth: usize,
    member_receiver_bindings: HashSet<Symbol>,
    /// Names assigned somewhere after an uninitialized `let x: T;` in the
    /// same statement list (definite-assignment approximation).
    assigned_later: HashSet<Symbol>,
    awaited_bindings: HashSet<Symbol>,
    /// Names bound by a `catch (e)` clause. `typeof e` reports `"object"`
    /// for these (real JavaScript throws an `Error` object), even though
    /// the binding is stored as the tagged error *string* internally --
    /// see `statements/lowering.rs`'s catch handling.
    catch_bindings: HashSet<Symbol>,
    promise_catch_bindings: HashSet<Symbol>,
    promise_catch_parameter: Option<Symbol>,
    loop_depth: usize,
    /// Emitted HIR depths for source breakable statements (iterations and switches).
    source_break_depths: Vec<usize>,
    /// Emitted HIR depths for source iterations only.
    source_continue_depths: Vec<usize>,
    labels: Vec<(Symbol, usize, bool)>,
    super_initializer: Option<(Symbol, HirType, Symbol)>,
    /// `<K, V>` / `<T>` of an `extends Map`/`Set` clause, replayed on the
    /// synthetic `new Map<K, V>(...)` that `super(...)` lowers to.
    native_base_type_args: Option<Box<swc_ecma_ast::TsTypeParamInstantiation>>,
    class_static_context: bool,
    class_context: Option<Symbol>,
    unbound_this_context: bool,
    non_arrow_receiver: Option<(HirType, Symbol)>,
    active_non_arrow_this: Option<(Symbol, HirType)>,
    /// A contextual type hint for the *next* call expression `lower_call`
    /// handles, consumed (taken, not just read) as its very first action
    /// so it can never leak into a nested/argument call's own inference --
    /// see `lower_expr_with_expected_type`'s doc comment for why this
    /// exists and how it stays scoped to exactly one call.
    expected_return_hint: Option<HirType>,
    /// Source-level expected return type for generic inference. Unlike the
    /// physical hint above, this is never used to choose a runtime ABI.
    expected_source_return_hint: Option<HirType>,
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
    /// One-shot: declared `Map`/`Set`/`WeakMap`/`WeakSet` type for a type-argument-less `new` of it.
    expected_collection_hint: Option<HirType>,
    /// Addresses of the argument lists of `const u = new Uint8Array(n)` declarations whose variable
    /// is only indexed or measured; only those constructions become a native byte store.
    native_u8_args: HashSet<usize>,
    /// One-shot: the next object literal lowered is an `any` literal, so its array-literal property
    /// values (recursively through nested object literals) are `Array<Json>`: an `any` array takes any
    /// element type later (`h.list.push("x")`), which a typed native array cannot hold.
    any_object_literal: bool,
    /// One-shot: the next lowered arrow is a callback that native array/Map/Set iteration
    /// calls with `Json` elements, so an `any` parameter it mutates stays a (shared) `Json`
    /// instead of becoming a live `JsValue` handle.
    native_iteration_callback: bool,
    /// The next arrow lowered is the initializer of a `const f = (x: any) => ..`: a stored function
    /// value keeps `Json` parameters (it may be handed to native iteration), not live `JsValue` ones.
    stored_arrow_initializer: bool,
    /// Set while lowering a live (`any`-context) object literal: unannotated callback
    /// parameters inside it are contextually `any`, as in TypeScript.
    implicit_any_callbacks: bool,
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
