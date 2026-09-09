/// Lowers one function body while retaining its typed lexical scope.
struct FnLowerer<'a> {
    scope: HashMap<Symbol, HirType>,
    immutable_bindings: HashSet<Symbol>,
    narrowings: HashMap<Symbol, HirType>,
    nullable_narrowings: HashMap<Symbol, HirType>,
    nullish_narrowings: HashMap<Symbol, HirType>,
    json_narrowings: HashMap<Symbol, HirType>,
    union_narrowings: HashMap<Symbol, (Vec<usize>, Vec<HirType>)>,
    union_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    array_element_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    nested_array_discriminants: HashMap<Symbol, NestedArrayDiscriminants>,
    object_array_property_discriminants: HashMap<Symbol, ObjectArrayPropertyDiscriminants>,
    object_function_property_discriminants: HashMap<Symbol, ObjectFunctionPropertyDiscriminants>,
    destructured_union_correlations: HashMap<Symbol, DestructuredUnionCorrelation>,
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
    generic_arrow_self_names: HashMap<Symbol, Symbol>,
    generic_named_templates: HashMap<Symbol, Symbol>,
    native_method_values: HashMap<Symbol, NativeMethodValue>,
    native_class_aliases: HashMap<Symbol, HirType>,
    member_receiver_bindings: HashSet<Symbol>,
    awaited_bindings: HashSet<Symbol>,
    loop_depth: usize,
    labels: Vec<(Symbol, usize, bool)>,
    super_initializer: Option<(Symbol, HirType, Symbol)>,
    class_static_context: bool,
    class_context: Option<Symbol>,
    unbound_this_context: bool,
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
    generator_yields: Option<(Symbol, HirType)>,
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
