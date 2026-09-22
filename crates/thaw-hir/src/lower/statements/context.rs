impl<'a> FnLowerer<'a> {
    fn expression_never_returns(&self, expr: &Expr) -> bool {
        let Expr::Call(call) = expr else {
            return false;
        };
        let Callee::Expr(callee) = &call.callee else {
            return false;
        };
        let Expr::Ident(callee) = callee.as_ref() else {
            return false;
        };
        self.signatures
            .get(&self.resolve_binding(callee.sym.as_ref()))
            .and_then(|signature| signature.generic_return_type.as_deref())
            .is_some_and(|ty| {
                matches!(
                    ty,
                    TsType::TsKeywordType(keyword)
                        if keyword.kind == swc_ecma_ast::TsKeywordTypeKind::TsNeverKeyword
                )
            })
    }

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
            json_narrowings: HashMap::new(),
            exception_object_narrowings: HashMap::new(),
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
            sparse_arrays: HashSet::new(),
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
            native_class_aliases: HashMap::new(),
            member_receiver_bindings: HashSet::new(),
            awaited_bindings: HashSet::new(),
            catch_bindings: HashSet::new(),
            promise_catch_bindings: HashSet::new(),
            promise_catch_parameter: None,
            loop_depth: 0,
            labels: Vec::new(),
            super_initializer: None,
            class_static_context: false,
            class_context: None,
            unbound_this_context: false,
            expected_return_hint: None,
            expected_arrow_return_hint: None,
            generator_yields: None,
            generator_finalizers: HashMap::new(),
        }
    }

    fn resolve_binding(&self, source_name: &str) -> Symbol {
        self.bindings
            .get(source_name)
            .and_then(|names| names.last())
            .cloned()
            .unwrap_or_else(|| source_name.to_string())
    }

    fn mark_array_parameter(&mut self, name: &str, ty: &HirType) {
        if matches!(ty, HirType::Array(_)) {
            self.sparse_arrays.insert(name.to_string());
        }
    }

    fn expression_may_be_sparse_array(&self, expression: &Expr) -> bool {
        match expression {
            Expr::Array(array) => array.elems.iter().any(|element| {
                element.as_ref().is_none_or(|element| {
                    element.spread.is_some()
                        && self.expression_may_be_sparse_array(&element.expr)
                })
            }),
            Expr::Ident(ident) => {
                let resolved = self.resolve_binding(ident.sym.as_ref());
                self.sparse_arrays.contains(&resolved)
            }
            Expr::Call(call) => match call.callee.as_expr().map(Box::as_ref) {
                Some(Expr::Member(member))
                    if matches!(
                        &member.prop,
                        MemberProp::Ident(property)
                            if matches!(property.sym.as_ref(), "slice" | "concat" | "map")
                    ) => self.expression_may_be_sparse_array(&member.obj),
                Some(Expr::Ident(callee)) => self
                    .signatures
                    .get(callee.sym.as_ref())
                    .is_some_and(|signature| signature.returns_sparse_array),
                _ => false,
            },
            Expr::Paren(paren) => self.expression_may_be_sparse_array(&paren.expr),
            Expr::TsAs(assertion) => self.expression_may_be_sparse_array(&assertion.expr),
            Expr::TsSatisfies(assertion) => self.expression_may_be_sparse_array(&assertion.expr),
            Expr::TsNonNull(assertion) => self.expression_may_be_sparse_array(&assertion.expr),
            _ => false,
        }
    }
}

include!("../flow_metadata.rs");
