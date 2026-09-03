impl<'a> FnLowerer<'a> {
    fn stmt_is_iteration(stmt: &Stmt) -> bool {
        match stmt {
            Stmt::While(_) | Stmt::DoWhile(_) | Stmt::For(_) | Stmt::ForIn(_) | Stmt::ForOf(_) => {
                true
            }
            Stmt::Labeled(labeled) => Self::stmt_is_iteration(&labeled.body),
            _ => false,
        }
    }

    fn new(
        signatures: &'a HashMap<Symbol, FnSignature>,
        interfaces: &'a HashMap<Symbol, HirType>,
        generic_interfaces: &'a GenericInterfaces<'a>,
        enum_values: &'a EnumValues,
        enum_reverse_values: &'a EnumReverseValues,
        ret_type: HirType,
        call_constraints: Option<&'a RefCell<Vec<CallConstraint>>>,
    ) -> Self {
        Self {
            scope: HashMap::new(),
            immutable_bindings: HashSet::new(),
            narrowings: HashMap::new(),
            nullable_narrowings: HashMap::new(),
            nullish_narrowings: HashMap::new(),
            union_narrowings: HashMap::new(),
            union_discriminants: HashMap::new(),
            array_element_discriminants: HashMap::new(),
            nested_array_discriminants: HashMap::new(),
            object_array_property_discriminants: HashMap::new(),
            object_function_property_discriminants: HashMap::new(),
            destructured_union_correlations: HashMap::new(),
            destructuring_default_types: HashMap::new(),
            function_value_discriminants: HashMap::new(),
            function_value_array_discriminants: HashMap::new(),
            function_value_nested_array_discriminants: HashMap::new(),
            function_value_object_array_property_discriminants: HashMap::new(),
            function_value_object_function_property_discriminants: HashMap::new(),
            bindings: HashMap::new(),
            used_hir_bindings: HashSet::new(),
            next_binding: 0,
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            ret_type,
            call_constraints,
            generic_call_returns: HashMap::new(),
            generic_arrows: HashMap::new(),
            generic_arrow_self_names: HashMap::new(),
            generic_named_templates: HashMap::new(),
            native_method_values: HashMap::new(),
            loop_depth: 0,
            labels: Vec::new(),
            super_initializer: None,
            class_static_context: false,
            class_context: None,
            unbound_this_context: false,
            expected_return_hint: None,
        }
    }

    fn resolve_binding(&self, source_name: &str) -> Symbol {
        self.bindings
            .get(source_name)
            .and_then(|names| names.last())
            .cloned()
            .unwrap_or_else(|| source_name.to_string())
    }
}

include!("../flow_metadata.rs");
