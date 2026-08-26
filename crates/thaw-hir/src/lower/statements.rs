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

include!("flow_metadata.rs");

impl<'a> FnLowerer<'a> {
    fn bind_local(&mut self, source_name: &str, ty: HirType) -> Symbol {
        let hir_name = if self.scope.contains_key(source_name)
            || self.used_hir_bindings.contains(source_name)
        {
            loop {
                let name = format!("{source_name}__thaw_{}", self.next_binding);
                self.next_binding += 1;
                if !self.scope.contains_key(&name) && !self.used_hir_bindings.contains(&name) {
                    break name;
                }
            }
        } else {
            source_name.to_string()
        };
        self.scope.insert(hir_name.clone(), ty);
        self.used_hir_bindings.insert(hir_name.clone());
        self.bindings
            .entry(source_name.to_string())
            .or_default()
            .push(hir_name.clone());
        hir_name
    }

    fn lower_scoped_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let saved = self.bindings.clone();
        let saved_narrowings = self.narrowings.clone();
        let saved_nullable_narrowings = self.nullable_narrowings.clone();
        let saved_nullish_narrowings = self.nullish_narrowings.clone();
        let saved_union_narrowings = self.union_narrowings.clone();
        let saved_generic_arrows = self.generic_arrows.clone();
        let saved_generic_arrow_self_names = self.generic_arrow_self_names.clone();
        let saved_generic_named_templates = self.generic_named_templates.clone();
        let saved_native_method_values = self.native_method_values.clone();
        let lowered = self.lower_stmts(stmts);
        self.bindings = saved;
        self.narrowings = saved_narrowings;
        self.nullable_narrowings = saved_nullable_narrowings;
        self.nullish_narrowings = saved_nullish_narrowings;
        self.union_narrowings = saved_union_narrowings;
        self.generic_arrows = saved_generic_arrows;
        self.generic_arrow_self_names = saved_generic_arrow_self_names;
        self.generic_named_templates = saved_generic_named_templates;
        self.native_method_values = saved_native_method_values;
        lowered
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let mut out = Vec::new();
        for stmt in stmts {
            out.extend(self.lower_stmt_seq(stmt)?);
            if let Stmt::If(if_stmt) = stmt {
                if if_stmt.alt.is_none() && Self::stmt_definitely_exits(&if_stmt.cons) {
                    if let Some((name, payload, present_when_true, absence_kind)) =
                        self.optional_undefined_narrowing(&if_stmt.test)
                    {
                        if !present_when_true {
                            match absence_kind {
                                0 => {
                                    self.narrowings.insert(name, payload);
                                }
                                1 => {
                                    self.nullable_narrowings.insert(name, payload);
                                }
                                2 => {
                                    self.nullish_narrowings.insert(name, payload);
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                    if let Some((targets, equal_when_true, complement)) =
                        self.union_narrowing(&if_stmt.test)
                    {
                        for target in targets {
                            let continuing = if equal_when_true {
                                if complement {
                                    target
                                        .allowed
                                        .into_iter()
                                        .filter(|index| !target.matching.contains(index))
                                        .collect::<Vec<_>>()
                                } else {
                                    target.allowed
                                }
                            } else {
                                target.matching
                            };
                            if !continuing.is_empty() {
                                self.union_narrowings
                                    .insert(target.name, (continuing, target.elements));
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    fn stmt_definitely_exits(stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Return(_) | Stmt::Throw(_) => true,
            Stmt::Block(block) => block.stmts.last().is_some_and(Self::stmt_definitely_exits),
            Stmt::If(if_stmt) => if_stmt.alt.as_ref().is_some_and(|alternative| {
                Self::stmt_definitely_exits(&if_stmt.cons)
                    && Self::stmt_definitely_exits(alternative)
            }),
            _ => false,
        }
    }

    fn switch_case_prevents_fallthrough(statements: &[Stmt]) -> bool {
        let Some(last) = statements.last() else {
            return false;
        };
        match last {
            Stmt::Break(break_stmt) => break_stmt.label.is_none(),
            Stmt::Return(_) | Stmt::Throw(_) => true,
            Stmt::Block(block) => Self::switch_case_prevents_fallthrough(&block.stmts),
            Stmt::If(if_stmt) => if_stmt.alt.as_ref().is_some_and(|alternative| {
                Self::stmt_definitely_exits(&if_stmt.cons)
                    && Self::stmt_definitely_exits(alternative)
            }),
            _ => false,
        }
    }

    fn infer_return_type(&self, body: &[HirStmt]) -> Result<HirType, String> {
        fn collect<'a>(stmts: &'a [HirStmt], out: &mut Vec<&'a HirExpr>, bare: &mut bool) {
            for stmt in stmts {
                match stmt {
                    HirStmt::Return(Some(value)) => out.push(value),
                    HirStmt::Return(None) => *bare = true,
                    HirStmt::If(_, then_body, else_body) => {
                        collect(then_body, out, bare);
                        collect(else_body, out, bare);
                    }
                    HirStmt::While(_, body) => collect(body, out, bare),
                    HirStmt::Try(body, _, catch_body) => {
                        collect(body, out, bare);
                        collect(catch_body, out, bare);
                    }
                    _ => {}
                }
            }
        }

        let mut values = Vec::new();
        let mut bare = false;
        collect(body, &mut values, &mut bare);
        if values.is_empty() {
            return Ok(HirType::Void);
        }
        if bare {
            return Err("function mixes value-returning and bare `return` statements".into());
        }
        let first = self.infer_expr_type(values[0])?;
        if first == HirType::Dynamic {
            return Ok(HirType::Dynamic);
        }
        for value in &values[1..] {
            let ty = self.infer_expr_type(value)?;
            if ty == HirType::Dynamic {
                return Ok(HirType::Dynamic);
            }
            if ty != first {
                return Err(format!(
                    "function returns incompatible types {first:?} and {ty:?}"
                ));
            }
        }
        Ok(first)
    }

    /// Normalizes a `for`/`while`/`if` body, which SWC represents as a
    /// single `Stmt` (either a `{ ... }` block or one bare statement), into
    /// a flat HIR statement list.
    fn lower_body(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            Stmt::Block(block) => self.lower_scoped_stmts(&block.stmts),
            other => {
                let saved = self.bindings.clone();
                let lowered = self.lower_stmt_seq(other);
                self.bindings = saved;
                lowered
            }
        }
    }

    fn lower_loop_body(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        self.loop_depth += 1;
        let result = self.lower_body(stmt);
        self.loop_depth -= 1;
        result
    }

    fn lower_body_with_optional_narrowing(
        &mut self,
        stmt: &Stmt,
        narrowing: Option<&(Symbol, HirType, u8)>,
    ) -> Result<Vec<HirStmt>, String> {
        let saved = self.narrowings.clone();
        let saved_nullable = self.nullable_narrowings.clone();
        let saved_nullish = self.nullish_narrowings.clone();
        if let Some((name, payload, absence_kind)) = narrowing {
            match absence_kind {
                0 => {
                    self.narrowings.insert(name.clone(), payload.clone());
                }
                1 => {
                    self.nullable_narrowings
                        .insert(name.clone(), payload.clone());
                }
                2 => {
                    self.nullish_narrowings
                        .insert(name.clone(), payload.clone());
                }
                _ => unreachable!(),
            }
        }
        let lowered = self.lower_body(stmt);
        self.narrowings = saved;
        self.nullable_narrowings = saved_nullable;
        self.nullish_narrowings = saved_nullish;
        lowered
    }

    fn lower_expr_with_optional_narrowing(
        &mut self,
        expr: &Expr,
        narrowing: Option<&(Symbol, HirType, u8)>,
    ) -> Result<HirExpr, String> {
        let saved = self.narrowings.clone();
        let saved_nullable = self.nullable_narrowings.clone();
        let saved_nullish = self.nullish_narrowings.clone();
        if let Some((name, payload, absence_kind)) = narrowing {
            match absence_kind {
                0 => {
                    self.narrowings.insert(name.clone(), payload.clone());
                }
                1 => {
                    self.nullable_narrowings
                        .insert(name.clone(), payload.clone());
                }
                2 => {
                    self.nullish_narrowings
                        .insert(name.clone(), payload.clone());
                }
                _ => unreachable!(),
            }
        }
        let lowered = self.lower_expr(expr);
        self.narrowings = saved;
        self.nullable_narrowings = saved_nullable;
        self.nullish_narrowings = saved_nullish;
        lowered
    }

    fn lower_expr_with_union_narrowing(
        &mut self,
        expr: &Expr,
        union: Option<&[UnionNarrowingTarget]>,
        optional: Option<&(Symbol, HirType, u8)>,
    ) -> Result<HirExpr, String> {
        let saved = self.union_narrowings.clone();
        if let Some(targets) = union {
            for target in targets {
                self.union_narrowings.insert(
                    target.name.clone(),
                    (target.matching.clone(), target.elements.clone()),
                );
            }
        }
        let lowered = self.lower_expr_with_optional_narrowing(expr, optional);
        self.union_narrowings = saved;
        lowered
    }

    /// Returns the optional binding tested by an undefined comparison and
    /// whether its payload is present in the true branch.
    fn optional_undefined_narrowing(&self, expr: &Expr) -> Option<(Symbol, HirType, bool, u8)> {
        if let Expr::Paren(paren) = expr {
            return self.optional_undefined_narrowing(&paren.expr);
        }
        if let Expr::Unary(unary) = expr {
            if unary.op == swc_ecma_ast::UnaryOp::Bang {
                return self
                    .optional_undefined_narrowing(&unary.arg)
                    .map(|(name, payload, present, nullable)| (name, payload, !present, nullable));
            }
            return None;
        }
        let Expr::Bin(binary) = expr else {
            return None;
        };
        if binary.op == BinaryOp::LogicalAnd {
            return self
                .optional_undefined_narrowing(&binary.left)
                .filter(|(_, _, present, _)| *present);
        }
        if binary.op == BinaryOp::LogicalOr {
            return self
                .optional_undefined_narrowing(&binary.left)
                .filter(|(_, _, present, _)| !*present);
        }
        let present_when_true = match binary.op {
            BinaryOp::NotEqEq | BinaryOp::NotEq => true,
            BinaryOp::EqEqEq | BinaryOp::EqEq => false,
            _ => return None,
        };
        let typeof_ident = match (binary.left.as_ref(), binary.right.as_ref()) {
            (Expr::Unary(unary), Expr::Lit(Lit::Str(value)))
                if unary.op == UnaryOp::TypeOf && value.value == *"undefined" =>
            {
                match unary.arg.as_ref() {
                    Expr::Ident(ident) => Some(ident),
                    _ => None,
                }
            }
            (Expr::Lit(Lit::Str(value)), Expr::Unary(unary))
                if unary.op == UnaryOp::TypeOf && value.value == *"undefined" =>
            {
                match unary.arg.as_ref() {
                    Expr::Ident(ident) => Some(ident),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(ident) = typeof_ident {
            let name = self.resolve_binding(ident.sym.as_ref());
            let HirType::Optional(payload) = self.scope.get(&name)? else {
                return None;
            };
            return Some((name, payload.as_ref().clone(), present_when_true, 0));
        }
        let (ident, nullable) = match (binary.left.as_ref(), binary.right.as_ref()) {
            (Expr::Ident(value), Expr::Ident(undefined)) if undefined.sym == *"undefined" => {
                (value, false)
            }
            (Expr::Ident(undefined), Expr::Ident(value)) if undefined.sym == *"undefined" => {
                (value, false)
            }
            (Expr::Ident(value), Expr::Lit(Lit::Null(_))) => (value, true),
            (Expr::Lit(Lit::Null(_)), Expr::Ident(value)) => (value, true),
            _ => return None,
        };
        if !nullable && self.scope.contains_key("undefined") {
            return None;
        }
        let name = self.resolve_binding(ident.sym.as_ref());
        let (payload, absence_kind) = match (self.scope.get(&name)?, nullable) {
            (HirType::Optional(payload), false) => (payload, 0),
            (HirType::Nullable(payload), true) => (payload, 1),
            (HirType::Nullish(payload), _)
                if matches!(binary.op, BinaryOp::EqEq | BinaryOp::NotEq) =>
            {
                (payload, 2)
            }
            _ => return None,
        };
        Some((
            name,
            payload.as_ref().clone(),
            present_when_true,
            absence_kind,
        ))
    }

    fn union_typeof_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let Expr::Bin(binary) = expr else { return None };
        let equal_when_true = match binary.op {
            BinaryOp::EqEqEq | BinaryOp::EqEq => true,
            BinaryOp::NotEqEq | BinaryOp::NotEq => false,
            _ => return None,
        };
        let (target, type_name) = match (binary.left.as_ref(), binary.right.as_ref()) {
            (Expr::Unary(unary), Expr::Lit(Lit::Str(name))) if unary.op == UnaryOp::TypeOf => {
                (unary.arg.as_ref(), name.value.to_string_lossy())
            }
            (Expr::Lit(Lit::Str(name)), Expr::Unary(unary)) if unary.op == UnaryOp::TypeOf => {
                (unary.arg.as_ref(), name.value.to_string_lossy())
            }
            _ => return None,
        };
        let (ident, property) = match target {
            Expr::Ident(ident) => (ident, None),
            Expr::Member(member) => {
                let Expr::Ident(ident) = member.obj.as_ref() else {
                    return None;
                };
                (ident, Some(member_property_name(&member.prop)?))
            }
            _ => return None,
        };
        let name = self.resolve_binding(ident.sym.as_ref());
        let HirType::Union(elements) = self.scope.get(&name)? else {
            return None;
        };
        let allowed = self
            .union_narrowings
            .get(&name)
            .map(|(allowed, _)| allowed.clone())
            .unwrap_or_else(|| (0..elements.len()).collect());
        let matching = allowed
            .iter()
            .copied()
            .filter(|index| {
                let narrowed = match (&elements[*index], property.as_deref()) {
                    (element, None) => element,
                    (HirType::Object(fields), Some(property)) => {
                        let Some((_, field)) = fields.iter().find(|(name, _)| name == property)
                        else {
                            return false;
                        };
                        field
                    }
                    (_, Some(_)) => return false,
                };
                native_typeof_name(narrowed) == Some(type_name.as_ref())
            })
            .collect::<Vec<_>>();
        (!matching.is_empty()).then(|| {
            (
                vec![UnionNarrowingTarget {
                    name,
                    matching,
                    allowed,
                    elements: elements.clone(),
                }],
                equal_when_true,
                true,
            )
        })
    }

    fn union_member_equality_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        if let Expr::Paren(paren) = expr {
            return self.union_member_equality_narrowing(&paren.expr);
        }
        if let Expr::Unary(unary) = expr {
            if unary.op == UnaryOp::Bang {
                return self
                    .union_member_equality_narrowing(&unary.arg)
                    .map(|(targets, equal, complement)| (targets, !equal, complement));
            }
            return None;
        }
        let Expr::Bin(binary) = expr else {
            return None;
        };
        let equal_when_true = match binary.op {
            BinaryOp::EqEqEq => true,
            BinaryOp::NotEqEq => false,
            _ => return None,
        };
        let literal_value = |value: &Expr| match value {
            Expr::Lit(Lit::Num(value)) => Some(HirLit::F64(value.value)),
            Expr::Lit(Lit::Str(value)) => {
                Some(HirLit::Str(value.value.to_string_lossy().into_owned()))
            }
            Expr::Lit(Lit::Bool(value)) => Some(HirLit::Bool(value.value)),
            _ => None,
        };
        let property_target = |value: &Expr| {
            let Expr::Member(member) = value else {
                return None;
            };
            let Expr::Ident(identifier) = member.obj.as_ref() else {
                return None;
            };
            Some((
                identifier.sym.to_string(),
                member_property_name(&member.prop)?,
            ))
        };
        let property_comparison = property_target(&binary.left)
            .zip(literal_value(&binary.right))
            .or_else(|| property_target(&binary.right).zip(literal_value(&binary.left)));
        if let Some(((identifier, property), literal)) = property_comparison {
            let name = self.resolve_binding(&identifier);
            let HirType::Union(elements) = self.scope.get(&name)? else {
                return None;
            };
            let values = self.union_discriminants.get(&name)?.get(&property)?;
            if values.len() != elements.len() {
                return None;
            }
            let allowed = self
                .union_narrowings
                .get(&name)
                .map(|(allowed, _)| allowed.clone())
                .unwrap_or_else(|| (0..elements.len()).collect());
            let matching = allowed
                .iter()
                .copied()
                .filter(|index| values[*index].as_ref() == Some(&literal))
                .collect::<Vec<_>>();
            return (!matching.is_empty()).then(|| {
                (
                    vec![UnionNarrowingTarget {
                        name,
                        matching,
                        allowed,
                        elements: elements.clone(),
                    }],
                    equal_when_true,
                    true,
                )
            });
        }
        let literal_type = |value: &Expr| match value {
            Expr::Lit(Lit::Num(_)) => Some((HirType::F64, false)),
            Expr::Lit(Lit::Str(_)) => Some((HirType::Str, false)),
            Expr::Lit(Lit::Bool(_)) => Some((HirType::Bool, false)),
            Expr::Lit(Lit::Null(_)) => Some((HirType::Null, true)),
            Expr::Ident(identifier)
                if identifier.sym == *"undefined" && !self.scope.contains_key("undefined") =>
            {
                Some((HirType::Undefined, true))
            }
            _ => None,
        };
        let (identifier, (member, complement)) =
            if let Expr::Ident(identifier) = binary.left.as_ref() {
                if let Some(member) = literal_type(&binary.right) {
                    (identifier, member)
                } else if let Expr::Ident(identifier) = binary.right.as_ref() {
                    (identifier, literal_type(&binary.left)?)
                } else {
                    return None;
                }
            } else if let Expr::Ident(identifier) = binary.right.as_ref() {
                (identifier, literal_type(&binary.left)?)
            } else {
                return None;
            };
        let name = self.resolve_binding(identifier.sym.as_ref());
        if let Some(correlation) = self.destructured_union_correlations.get(&name) {
            let literal = literal_value(if binary.left.as_ident() == Some(identifier) {
                binary.right.as_ref()
            } else {
                binary.left.as_ref()
            })?;
            let matching_sources = correlation
                .literals
                .iter()
                .enumerate()
                .filter_map(|(index, value)| (value.as_ref() == Some(&literal)).then_some(index))
                .collect::<Vec<_>>();
            if matching_sources.is_empty() {
                return None;
            }
            let targets = correlation
                .targets
                .iter()
                .filter_map(|target| {
                    let allowed = self
                        .union_narrowings
                        .get(&target.name)
                        .map(|(allowed, _)| allowed.clone())
                        .unwrap_or_else(|| (0..target.elements.len()).collect());
                    let mut matching = Vec::new();
                    for source in &matching_sources {
                        for index in &target.source_members[*source] {
                            if allowed.contains(index) && !matching.contains(index) {
                                matching.push(*index);
                            }
                        }
                    }
                    (!matching.is_empty()).then(|| UnionNarrowingTarget {
                        name: target.name.clone(),
                        matching,
                        allowed,
                        elements: target.elements.clone(),
                    })
                })
                .collect::<Vec<_>>();
            return (!targets.is_empty()).then_some((targets, equal_when_true, true));
        }
        let HirType::Union(elements) = self.scope.get(&name)? else {
            return None;
        };
        let allowed = self
            .union_narrowings
            .get(&name)
            .map(|(allowed, _)| allowed.clone())
            .unwrap_or_else(|| (0..elements.len()).collect());
        let index = elements.iter().position(|element| element == &member)?;
        if !allowed.contains(&index) {
            return None;
        }
        Some((
            vec![UnionNarrowingTarget {
                name,
                matching: vec![index],
                allowed,
                elements: elements.clone(),
            }],
            equal_when_true,
            complement,
        ))
    }

    fn union_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        if let Expr::Paren(parenthesized) = expr {
            return self.union_narrowing(&parenthesized.expr);
        }
        if let Expr::Unary(unary) = expr {
            if unary.op == UnaryOp::Bang {
                return self
                    .union_narrowing(&unary.arg)
                    .map(|(targets, equal, complement)| (targets, !equal, complement));
            }
        }
        if let Expr::Bin(binary) = expr {
            if matches!(binary.op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) {
                let (targets, equal, complement) = self.union_narrowing(&binary.left)?;
                let branch_targets = |truth: bool| {
                    targets
                        .iter()
                        .map(|target| UnionNarrowingTarget {
                            name: target.name.clone(),
                            matching: if truth == equal {
                                target.matching.clone()
                            } else if complement {
                                target
                                    .allowed
                                    .iter()
                                    .filter(|index| !target.matching.contains(index))
                                    .copied()
                                    .collect()
                            } else {
                                target.allowed.clone()
                            },
                            allowed: target.allowed.clone(),
                            elements: target.elements.clone(),
                        })
                        .collect()
                };
                return if binary.op == BinaryOp::LogicalAnd {
                    Some((branch_targets(true), true, false))
                } else {
                    Some((branch_targets(false), false, false))
                };
            }
        }
        self.union_typeof_narrowing(expr)
            .or_else(|| self.union_member_equality_narrowing(expr))
    }

    fn lower_body_with_union_narrowing(
        &mut self,
        stmt: &Stmt,
        narrowing: Option<&[UnionNarrowingTarget]>,
        optional: Option<&(Symbol, HirType, u8)>,
    ) -> Result<Vec<HirStmt>, String> {
        let saved = self.union_narrowings.clone();
        if let Some(targets) = narrowing {
            for target in targets {
                self.union_narrowings.insert(
                    target.name.clone(),
                    (target.matching.clone(), target.elements.clone()),
                );
            }
        }
        let lowered = self.lower_body_with_optional_narrowing(stmt, optional);
        self.union_narrowings = saved;
        lowered
    }

    fn lower_stmt_seq(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            // Empty statements have no runtime effect. `debugger` only has an
            // observable effect when a JavaScript debugger is attached; a
            // native Thaw executable therefore treats it as a no-op.
            Stmt::Empty(_) | Stmt::Debugger(_) => Ok(Vec::new()),
            Stmt::Return(ret) => {
                let value = match &ret.arg {
                    Some(arg) => {
                        let value = self.lower_expr(arg)?;
                        if self.ret_type == HirType::Void {
                            if self.infer_expr_type(&value)? == HirType::Void
                                && contains_await(&value)
                            {
                                Some(value)
                            } else {
                                return Err("a void function cannot return a value".into());
                            }
                        } else {
                            Some(self.coerce_to_declared(&self.ret_type.clone(), value)?)
                        }
                    }
                    None => {
                        if !matches!(self.ret_type, HirType::Void | HirType::Dynamic) {
                            return Err(format!(
                                "bare `return` is not valid for return type {:?}",
                                self.ret_type
                            ));
                        }
                        None
                    }
                };
                Ok(vec![HirStmt::Return(value)])
            }
            Stmt::Expr(expr_stmt) => Ok(vec![HirStmt::Expr(self.lower_expr(&expr_stmt.expr)?)]),
            Stmt::Block(block) => self.lower_scoped_stmts(&block.stmts),
            Stmt::Decl(Decl::Var(var_decl)) => self.lower_var_decl(var_decl),

            Stmt::If(if_stmt) => {
                let narrowing = self.optional_undefined_narrowing(&if_stmt.test);
                let union_narrowing = self.union_narrowing(&if_stmt.test);
                let cond = self.lower_expr(&if_stmt.test)?;
                self.expect_type(&HirType::Bool, &cond, "if condition")?;
                let then_narrowing = narrowing
                    .as_ref()
                    .filter(|(_, _, present, _)| *present)
                    .map(|(name, payload, _, nullable)| {
                        (name.clone(), payload.clone(), *nullable)
                    });
                let else_narrowing = narrowing
                    .as_ref()
                    .filter(|(_, _, present, _)| !*present)
                    .map(|(name, payload, _, nullable)| {
                        (name.clone(), payload.clone(), *nullable)
                    });
                let branch_union = |truth: bool| {
                    union_narrowing.as_ref().map(|(targets, equal, complement)| {
                        targets
                            .iter()
                            .map(|target| UnionNarrowingTarget {
                                name: target.name.clone(),
                                matching: if truth == *equal {
                                    target.matching.clone()
                                } else if !complement {
                                    target.allowed.clone()
                                } else {
                                    target
                                        .allowed
                                        .iter()
                                        .filter(|index| !target.matching.contains(index))
                                        .copied()
                                        .collect()
                                },
                                allowed: target.allowed.clone(),
                                elements: target.elements.clone(),
                            })
                            .collect::<Vec<_>>()
                    })
                };
                let then_union = branch_union(true);
                let else_union = branch_union(false);
                let then_branch = self
                    .lower_body_with_union_narrowing(
                        &if_stmt.cons,
                        then_union.as_deref(),
                        then_narrowing.as_ref(),
                    )?;
                let else_branch = match &if_stmt.alt {
                    Some(alt) => self.lower_body_with_union_narrowing(
                        alt,
                        else_union.as_deref(),
                        else_narrowing.as_ref(),
                    )?,
                    None => Vec::new(),
                };
                Ok(vec![HirStmt::If(cond, then_branch, else_branch)])
            }

            Stmt::While(while_stmt) => {
                let cond = self.lower_expr(&while_stmt.test)?;
                self.expect_type(&HirType::Bool, &cond, "while condition")?;
                let body = self.lower_loop_body(&while_stmt.body)?;
                Ok(vec![HirStmt::While(cond, body)])
            }

            Stmt::DoWhile(do_while) => {
                let cond = self.lower_expr(&do_while.test)?;
                self.expect_type(&HirType::Bool, &cond, "do/while condition")?;
                let guard = HirStmt::If(cond, Vec::new(), vec![HirStmt::Break]);
                let mut body = self.lower_loop_body(&do_while.body)?;
                body = inject_do_while_guard_before_continue(body, &guard);
                body.push(guard);
                Ok(vec![HirStmt::While(
                    HirExpr::Lit(HirLit::Bool(true)),
                    body,
                )])
            }

            Stmt::Break(break_stmt) => {
                if let Some(label) = &break_stmt.label {
                    let name = label.sym.as_ref();
                    let (_, target_depth, _) = self
                        .labels
                        .iter()
                        .rev()
                        .find(|(candidate, _, _)| candidate == name)
                        .ok_or_else(|| format!("unknown break label `{name}`"))?;
                    return Ok(vec![HirStmt::BreakDepth(self.loop_depth - target_depth)]);
                }
                Ok(vec![HirStmt::Break])
            }

            Stmt::Continue(continue_stmt) => {
                if let Some(label) = &continue_stmt.label {
                    let name = label.sym.as_ref();
                    let (_, target_depth, continuable) = self
                        .labels
                        .iter()
                        .rev()
                        .find(|(candidate, _, _)| candidate == name)
                        .ok_or_else(|| format!("unknown continue label `{name}`"))?;
                    if !continuable {
                        return Err(format!("continue label `{name}` does not name a loop"));
                    }
                    return Ok(vec![HirStmt::ContinueDepth(
                        self.loop_depth - target_depth,
                    )]);
                }
                Ok(vec![HirStmt::Continue])
            }

            Stmt::Labeled(labeled) => {
                let name = labeled.label.sym.to_string();
                if self.labels.iter().any(|(candidate, _, _)| candidate == &name) {
                    return Err(format!("duplicate active label `{name}`"));
                }
                let is_loop = Self::stmt_is_iteration(&labeled.body);
                let target_depth = self.loop_depth + 1;
                self.labels.push((name, target_depth, is_loop));
                let lowered = if is_loop {
                    self.lower_stmt_seq(&labeled.body)
                } else {
                    self.loop_depth += 1;
                    let body_result = self.lower_body(&labeled.body);
                    self.loop_depth -= 1;
                    body_result.map(|mut body| {
                        body.push(HirStmt::Break);
                        vec![HirStmt::While(HirExpr::Lit(HirLit::Bool(true)), body)]
                    })
                };
                self.labels.pop();
                lowered
            }

            Stmt::For(for_stmt) => {
                let saved = self.bindings.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let mut out = Vec::new();
                    if let Some(init) = &for_stmt.init {
                        match init {
                            VarDeclOrExpr::VarDecl(var_decl) => {
                                out.extend(self.lower_var_decl(var_decl)?)
                            }
                            VarDeclOrExpr::Expr(expr) => {
                                out.push(HirStmt::Expr(self.lower_expr(expr)?))
                            }
                        }
                    }

                    let cond = match &for_stmt.test {
                        Some(test) => {
                            let cond = self.lower_expr(test)?;
                            self.expect_type(&HirType::Bool, &cond, "for condition")?;
                            cond
                        }
                        None => HirExpr::Lit(HirLit::Bool(true)),
                    };

                    let mut body = self.lower_loop_body(&for_stmt.body)?;
                    if let Some(update) = &for_stmt.update {
                        let update = self.lower_expr(update)?;
                        body = inject_for_update_before_continue(body, &update);
                        body.push(HirStmt::Expr(update));
                    }

                    out.push(HirStmt::While(cond, body));
                    Ok(out)
                })();
                self.bindings = saved;
                lowered
            }

            Stmt::ForOf(for_of) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let saved_correlations = self.destructured_union_correlations.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let item_discriminants =
                        self.expression_array_element_discriminants(&for_of.right);
                    let values = self.lower_expr(&for_of.right)?;
                    let HirType::Array(element) = self.infer_expr_type(&values)? else {
                        return Err("`for...of` currently requires a typed array".into());
                    };
                    let (item_type, await_item) = if for_of.is_await {
                        match element.as_ref() {
                            HirType::Promise(resolved) => (resolved.as_ref().clone(), true),
                            synchronous => (synchronous.clone(), false),
                        }
                    } else {
                        (element.as_ref().clone(), false)
                    };
                    let values_name = format!("__thaw_for_of_values_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(
                        values_name.clone(),
                        HirType::Array(element.clone()),
                    );
                    let index_name = format!("__thaw_for_of_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let item_value = || {
                        let indexed = HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(values_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element.as_ref().clone(),
                        );
                        if await_item {
                            HirExpr::AwaitPromise(Box::new(indexed), item_type.clone())
                        } else {
                            indexed
                        }
                    };
                    let item_stmts = match &for_of.left {
                        ForHead::VarDecl(decl) => {
                            let [declarator] = decl.decls.as_slice() else {
                                return Err("`for...of` requires exactly one loop binding".into());
                            };
                            if declarator.init.is_some() {
                                return Err(
                                    "`for...of` loop bindings cannot have an initializer".into(),
                                );
                            }
                            if let Pat::Ident(binding) = &declarator.name {
                                let item_ty = match &binding.type_ann {
                                    Some(annotation) => {
                                        let declared = lower_ts_type(
                                            &annotation.type_ann,
                                            self.interfaces,
                                            self.generic_interfaces,
                                        )?;
                                        if declared != item_type {
                                            return Err(format!(
                                                "`for...of` binding has type {declared:?}, expected {:?}",
                                                item_type
                                            ));
                                        }
                                        declared
                                    }
                                    None => item_type.clone(),
                                };
                                let source_name = binding.id.sym.to_string();
                                let item_name =
                                    format!("{source_name}__thaw_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(item_name.clone(), item_ty.clone());
                                self.bindings
                                    .entry(source_name)
                                    .or_default()
                                    .push(item_name.clone());
                                let declared_discriminants = binding.type_ann.as_ref().map(
                                    |annotation| {
                                        object_union_discriminants(
                                            &annotation.type_ann,
                                            self.generic_interfaces,
                                        )
                                    },
                                );
                                let discriminants = declared_discriminants
                                    .filter(|metadata| !metadata.is_empty())
                                    .or_else(|| item_discriminants.clone());
                                if let Some(discriminants) = discriminants {
                                    self.union_discriminants
                                        .insert(item_name.clone(), discriminants);
                                }
                                vec![HirStmt::Let(item_name, item_ty, item_value())]
                            } else if matches!(declarator.name, Pat::Object(_) | Pat::Array(_)) {
                                let temporary =
                                    format!("__thaw_for_of_item_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(temporary.clone(), item_type.clone());
                                let annotation = match &declarator.name {
                                    Pat::Object(pattern) => pattern.type_ann.as_ref(),
                                    Pat::Array(pattern) => pattern.type_ann.as_ref(),
                                    _ => None,
                                };
                                let declared_discriminants = annotation.map(|annotation| {
                                    object_union_discriminants(
                                        &annotation.type_ann,
                                        self.generic_interfaces,
                                    )
                                });
                                let discriminants = declared_discriminants
                                    .filter(|metadata| !metadata.is_empty())
                                    .or_else(|| item_discriminants.clone());
                                if let Some(discriminants) = discriminants {
                                    self.union_discriminants
                                        .insert(temporary.clone(), discriminants);
                                }
                                let mut statements = vec![HirStmt::Let(
                                    temporary.clone(),
                                    item_type.clone(),
                                    item_value(),
                                )];
                                self.lower_binding_pattern(
                                    &declarator.name,
                                    HirExpr::Var(temporary),
                                    &item_type,
                                    &mut statements,
                                )?;
                                statements
                            } else {
                                return Err("unsupported `for...of` binding pattern".into());
                            }
                        }
                        ForHead::Pat(pattern) => {
                            if let Pat::Ident(binding) = pattern.as_ref() {
                                let item_name = self.resolve_binding(binding.id.sym.as_ref());
                                let item_ty = self.scope.get(&item_name).cloned().ok_or_else(|| {
                                    format!("unknown `for...of` assignment target `{item_name}`")
                                })?;
                                if item_ty != item_type {
                                    return Err(format!(
                                        "`for...of` assignment target has type {item_ty:?}, expected {:?}",
                                        item_type
                                    ));
                                }
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    item_name,
                                    Box::new(item_value()),
                                ))]
                            } else if matches!(pattern.as_ref(), Pat::Object(_) | Pat::Array(_)) {
                                let temporary =
                                    format!("__thaw_for_of_item_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(temporary.clone(), item_type.clone());
                                if let Some(discriminants) = item_discriminants.clone() {
                                    self.union_discriminants
                                        .insert(temporary.clone(), discriminants);
                                }
                                let mut statements = vec![HirStmt::Let(
                                    temporary.clone(),
                                    item_type.clone(),
                                    item_value(),
                                )];
                                self.lower_assignment_pattern(
                                    pattern,
                                    HirExpr::Var(temporary),
                                    &item_type,
                                    &mut statements,
                                )?;
                                statements
                            } else {
                                return Err("unsupported `for...of` assignment pattern".into());
                            }
                        }
                        ForHead::UsingDecl(_) => {
                            return Err("`using` bindings in `for...of` are not supported".into())
                        }
                    };
                    let mut body = item_stmts;
                    body.extend(self.lower_loop_body(&for_of.body)?);
                    let update = HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    );
                    body = inject_for_update_before_continue(body, &update);
                    body.push(HirStmt::Expr(update));
                    Ok(vec![
                        HirStmt::Let(
                            values_name.clone(),
                            HirType::Array(element),
                            values,
                        ),
                        HirStmt::Let(
                            index_name.clone(),
                            HirType::F64,
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ),
                        HirStmt::While(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(HirExpr::Var(index_name)),
                                Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(values_name)))),
                            ),
                            body,
                        ),
                    ])
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                self.destructured_union_correlations = saved_correlations;
                lowered
            }

            Stmt::ForIn(for_in) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let object = self.lower_expr(&for_in.right)?;
                    let object_type = self.infer_expr_type(&object)?;
                    let keys = match &object_type {
                        HirType::Object(fields) => fields
                            .iter()
                            .map(|(name, _)| HirExpr::Lit(HirLit::Str(name.clone())))
                            .collect::<Vec<_>>(),
                        _ => {
                            return Err(
                                "`for...in` currently requires a fixed-shape object".into(),
                            )
                        }
                    };
                    let object_name = format!("__thaw_for_in_object_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(object_name.clone(), object_type.clone());
                    let keys_name = format!("__thaw_for_in_keys_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(
                        keys_name.clone(),
                        HirType::Array(Box::new(HirType::Str)),
                    );
                    let index_name = format!("__thaw_for_in_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let key_value = || {
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(keys_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            HirType::Str,
                        )
                    };
                    let binding_stmt = match &for_in.left {
                        ForHead::VarDecl(decl) => {
                            let [declarator] = decl.decls.as_slice() else {
                                return Err("`for...in` requires exactly one loop binding".into());
                            };
                            if declarator.init.is_some() {
                                return Err(
                                    "`for...in` loop bindings cannot have an initializer".into(),
                                );
                            }
                            let Pat::Ident(binding) = &declarator.name else {
                                return Err(
                                    "`for...in` requires an identifier loop binding".into(),
                                );
                            };
                            if let Some(annotation) = &binding.type_ann {
                                let declared = lower_ts_type(
                                    &annotation.type_ann,
                                    self.interfaces,
                                    self.generic_interfaces,
                                )?;
                                if declared != HirType::Str {
                                    return Err(format!(
                                        "`for...in` binding must be Str, got {declared:?}"
                                    ));
                                }
                            }
                            let source_name = binding.id.sym.to_string();
                            let binding_name =
                                format!("{source_name}__thaw_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(binding_name.clone(), HirType::Str);
                            self.bindings
                                .entry(source_name)
                                .or_default()
                                .push(binding_name.clone());
                            HirStmt::Let(binding_name, HirType::Str, key_value())
                        }
                        ForHead::Pat(pattern) => {
                            let Pat::Ident(binding) = pattern.as_ref() else {
                                return Err(
                                    "`for...in` assignment requires an identifier target".into(),
                                );
                            };
                            let binding_name = self.resolve_binding(binding.id.sym.as_ref());
                            let binding_type = self.scope.get(&binding_name).ok_or_else(|| {
                                format!("unknown `for...in` assignment target `{binding_name}`")
                            })?;
                            if binding_type != &HirType::Str {
                                return Err(format!(
                                    "`for...in` assignment target must be Str, got {binding_type:?}"
                                ));
                            }
                            HirStmt::Expr(HirExpr::Assign(
                                binding_name,
                                Box::new(key_value()),
                            ))
                        }
                        ForHead::UsingDecl(_) => {
                            return Err("`using` bindings in `for...in` are not supported".into())
                        }
                    };
                    let update = HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    );
                    let mut body = vec![binding_stmt];
                    body.extend(self.lower_loop_body(&for_in.body)?);
                    body = inject_for_update_before_continue(body, &update);
                    body.push(HirStmt::Expr(update));
                    Ok(vec![
                        HirStmt::Let(object_name, object_type, object),
                        HirStmt::Let(
                            keys_name.clone(),
                            HirType::Array(Box::new(HirType::Str)),
                            HirExpr::ArrayLit(keys),
                        ),
                        HirStmt::Let(
                            index_name.clone(),
                            HirType::F64,
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ),
                        HirStmt::While(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(HirExpr::Var(index_name)),
                                Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(keys_name)))),
                            ),
                            body,
                        ),
                    ])
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                lowered
            }

            Stmt::Switch(switch_stmt) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let discriminant = self.lower_expr(&switch_stmt.discriminant)?;
                    let discriminant_type = self.infer_expr_type(&discriminant)?;
                    let value_name = format!("__thaw_switch_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(value_name.clone(), discriminant_type.clone());
                    let selected_name = format!("__thaw_switch_selected_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(selected_name.clone(), HirType::F64);
                    let none = HirExpr::Lit(HirLit::F64(-1.0));
                    let case_count = switch_stmt.cases.len();
                    let default_index = switch_stmt
                        .cases
                        .iter()
                        .position(|case| case.test.is_none())
                        .unwrap_or(case_count);
                    let mut out = vec![
                        HirStmt::Let(value_name.clone(), discriminant_type.clone(), discriminant),
                        HirStmt::Let(selected_name.clone(), HirType::F64, none.clone()),
                    ];

                    for (index, case) in switch_stmt.cases.iter().enumerate() {
                        let Some(test) = &case.test else {
                            continue;
                        };
                        let test = self.lower_expr(test)?;
                        self.expect_type(&discriminant_type, &test, "switch case")?;
                        out.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(none.clone()),
                            ),
                            vec![HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::Var(value_name.clone())),
                                    Box::new(test),
                                ),
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    selected_name.clone(),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                ))],
                                Vec::new(),
                            )],
                            Vec::new(),
                        ));
                    }
                    out.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::Var(selected_name.clone())),
                            Box::new(none),
                        ),
                        vec![HirStmt::Expr(HirExpr::Assign(
                            selected_name.clone(),
                            Box::new(HirExpr::Lit(HirLit::F64(default_index as f64))),
                        ))],
                        Vec::new(),
                    ));

                    let exit = HirStmt::Expr(HirExpr::Assign(
                        selected_name.clone(),
                        Box::new(HirExpr::Lit(HirLit::F64(case_count as f64))),
                    ));
                    for (index, case) in switch_stmt.cases.iter().enumerate() {
                        let isolated_entry = index == 0
                            || Self::switch_case_prevents_fallthrough(
                                &switch_stmt.cases[index - 1].cons,
                            );
                        let case_union = if isolated_entry {
                            if let Some(test) = &case.test {
                                self.union_narrowing(&Expr::Bin(swc_ecma_ast::BinExpr {
                                    span: switch_stmt.span,
                                    op: BinaryOp::EqEqEq,
                                    left: switch_stmt.discriminant.clone(),
                                    right: test.clone(),
                                }))
                                .map(|(targets, equal, complement)| {
                                    targets
                                        .into_iter()
                                        .map(|target| UnionNarrowingTarget {
                                            matching: if equal {
                                                target.matching.clone()
                                            } else if complement {
                                                target
                                                    .allowed
                                                    .iter()
                                                    .filter(|member| {
                                                        !target.matching.contains(member)
                                                    })
                                                    .copied()
                                                    .collect()
                                            } else {
                                                target.allowed.clone()
                                            },
                                            ..target
                                        })
                                        .collect::<Vec<_>>()
                                })
                            } else {
                                let mut excluded = HashMap::<
                                    Symbol,
                                    (Vec<usize>, Vec<usize>, Vec<HirType>),
                                >::new();
                                for tested_case in &switch_stmt.cases {
                                    let Some(test) = &tested_case.test else {
                                        continue;
                                    };
                                    let Some((targets, equal, _)) = self.union_narrowing(
                                        &Expr::Bin(swc_ecma_ast::BinExpr {
                                            span: switch_stmt.span,
                                            op: BinaryOp::EqEqEq,
                                            left: switch_stmt.discriminant.clone(),
                                            right: test.clone(),
                                        }),
                                    ) else {
                                        continue;
                                    };
                                    if !equal {
                                        continue;
                                    }
                                    for target in targets {
                                        let entry = excluded.entry(target.name).or_insert_with(|| {
                                            (Vec::new(), target.allowed, target.elements)
                                        });
                                        for member in target.matching {
                                            if !entry.0.contains(&member) {
                                                entry.0.push(member);
                                            }
                                        }
                                    }
                                }
                                let targets = excluded
                                    .into_iter()
                                    .filter_map(|(name, (excluded, allowed, elements))| {
                                        let matching = allowed
                                            .iter()
                                            .filter(|member| !excluded.contains(member))
                                            .copied()
                                            .collect::<Vec<_>>();
                                        (!matching.is_empty()).then_some(UnionNarrowingTarget {
                                            name,
                                            matching,
                                            allowed,
                                            elements,
                                        })
                                    })
                                    .collect::<Vec<_>>();
                                (!targets.is_empty()).then_some(targets)
                            }
                        } else {
                            None
                        };
                        let saved_union_narrowings = self.union_narrowings.clone();
                        if let Some(targets) = &case_union {
                            for target in targets {
                                self.union_narrowings.insert(
                                    target.name.clone(),
                                    (target.matching.clone(), target.elements.clone()),
                                );
                            }
                        }
                        let lowered_case = self.lower_stmts(&case.cons);
                        self.union_narrowings = saved_union_narrowings;
                        let mut body = lowered_case?;
                        body = rewrite_switch_case_stmts(
                            body,
                            &selected_name,
                            index,
                            &exit,
                        );
                        body.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            vec![HirStmt::Expr(HirExpr::Assign(
                                selected_name.clone(),
                                Box::new(HirExpr::Lit(HirLit::F64((index + 1) as f64))),
                            ))],
                            Vec::new(),
                        ));
                        out.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            body,
                            Vec::new(),
                        ));
                    }
                    Ok(out)
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                lowered
            }

            Stmt::Throw(throw_stmt) => Ok(vec![HirStmt::Throw(self.lower_expr(&throw_stmt.arg)?)]),

            Stmt::Try(try_stmt) => {
                if try_stmt.handler.is_none() && try_stmt.finalizer.is_none() {
                    return Err("`try` needs a `catch` or `finally` block".into());
                }

                let mut body = self.lower_scoped_stmts(&try_stmt.block.stmts)?;
                let (catch_name, mut catch_body) = match &try_stmt.handler {
                    Some(handler) => {
                        let source_name = match &handler.param {
                            Some(Pat::Ident(binding)) => binding.id.sym.to_string(),
                            Some(_) => {
                                return Err(
                                    "only a simple identifier catch binding is supported".into(),
                                )
                            }
                            None => "_".to_string(),
                        };
                        let saved = self.bindings.clone();
                        let catch_name = self.bind_local(&source_name, HirType::Str);
                        let catch_body = self.lower_stmts(&handler.body.stmts)?;
                        self.bindings = saved;
                        (catch_name, catch_body)
                    }
                    None => {
                        let mut name = "__thaw_finally_exception".to_string();
                        while self.scope.contains_key(&name) {
                            name.push('_');
                        }
                        let name = self.bind_local(&name, HirType::Str);
                        let rethrow = HirStmt::Throw(HirExpr::Var(name.clone()));
                        (name, vec![rethrow])
                    }
                };

                let mut after_try = Vec::new();
                if let Some(finalizer) = &try_stmt.finalizer {
                    let finalizer = self.lower_scoped_stmts(&finalizer.stmts)?;
                    body = inject_finally_before_exits(body, &finalizer, false);
                    catch_body =
                        inject_finally_before_exits(catch_body, &finalizer, true);
                    after_try = finalizer;
                }
                let mut lowered = vec![HirStmt::Try(body, catch_name, catch_body)];
                lowered.extend(after_try);
                Ok(lowered)
            }

            other => Err(format!(
                "unsupported statement {other:?} (Phase 0/1/2 support return/expr/let/if/while/for/throw/try)"
            )),
        }
    }

    fn lower_var_decl(&mut self, var_decl: &VarDecl) -> Result<Vec<HirStmt>, String> {
        let mut statements = Vec::new();
        for decl in &var_decl.decls {
            if let Pat::Ident(binding) = &decl.name {
                let name = binding.id.sym.to_string();
                let init = decl
                    .init
                    .as_deref()
                    .ok_or_else(|| format!("`{name}` needs an initializer"))?;
                let correlated_alias_source = self.expression_identifier_alias_source(init);
                if let (Expr::Arrow(arrow), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if arrow.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let actual = self.generic_arrow_signature(arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name, arrow.clone());
                            continue;
                        }
                    }
                }
                if let (Expr::Fn(function), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if function.function.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let arrow = function_expression_as_arrow(function)?;
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name.clone(), arrow);
                            if named_function_is_recursive(function) {
                                self.generic_arrow_self_names.insert(
                                    hir_name,
                                    function.ident.as_ref().unwrap().sym.to_string(),
                                );
                            }
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        if let Some(actual) = self.signatures.get(identifier.sym.as_ref()) {
                            if !actual.generic_type_params.is_empty() {
                                self.validate_generic_callable_shape(
                                    &expected,
                                    actual,
                                    &callable_name,
                                )?;
                                let hir_name = self.bind_local(&name, HirType::Dynamic);
                                self.generic_named_templates
                                    .insert(hir_name, identifier.sym.to_string());
                                continue;
                            }
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        let source_name = self.resolve_binding(identifier.sym.as_ref());
                        if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name.clone(), arrow);
                            if let Some(self_name) =
                                self.generic_arrow_self_names.get(&source_name).cloned()
                            {
                                self.generic_arrow_self_names.insert(hir_name, self_name);
                            }
                            continue;
                        }
                        if let Some(target) =
                            self.generic_named_templates.get(&source_name).cloned()
                        {
                            let actual = self
                                .signatures
                                .get(&target)
                                .expect("named generic template target");
                            self.validate_generic_callable_shape(
                                &expected,
                                actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_named_templates.insert(hir_name, target);
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), None) = (init, binding.type_ann.as_ref()) {
                    let source_name = self.resolve_binding(identifier.sym.as_ref());
                    if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                        let hir_name = self.bind_local(&name, HirType::Dynamic);
                        self.generic_arrows.insert(hir_name.clone(), arrow);
                        if let Some(self_name) =
                            self.generic_arrow_self_names.get(&source_name).cloned()
                        {
                            self.generic_arrow_self_names.insert(hir_name, self_name);
                        }
                        continue;
                    }
                    if let Some(target) = self.generic_named_templates.get(&source_name).cloned() {
                        let hir_name = self.bind_local(&name, HirType::Dynamic);
                        self.generic_named_templates.insert(hir_name, target);
                        continue;
                    }
                }
                let annotated = binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        lower_ts_type(
                            &annotation.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                    })
                    .transpose()?;
                if let Expr::Fn(function) = init {
                    if let Some((hir_name, ty, value)) = self.lower_recursive_function_expression(
                        &name,
                        function,
                        annotated.as_ref(),
                    )? {
                        statements.push(HirStmt::Let(hir_name, ty, value));
                        continue;
                    }
                }
                if annotated.is_none()
                    && (matches!(init, Expr::Arrow(arrow) if arrow.type_params.is_some())
                        || matches!(init, Expr::Fn(function) if function.function.type_params.is_some()))
                {
                    let recursive_self = match init {
                        Expr::Fn(function) if named_function_is_recursive(function) => {
                            function.ident.as_ref().map(|name| name.sym.to_string())
                        }
                        _ => None,
                    };
                    let arrow = match init {
                        Expr::Arrow(arrow) => arrow.clone(),
                        Expr::Fn(function) => function_expression_as_arrow(function)?,
                        _ => unreachable!(),
                    };
                    if arrow.is_async || arrow.is_generator {
                        return Err(
                            "async and generator generic arrow variables are not supported".into(),
                        );
                    }
                    let hir_name = self.bind_local(&name, HirType::Dynamic);
                    self.generic_arrows.insert(hir_name.clone(), arrow);
                    if let Some(self_name) = recursive_self {
                        self.generic_arrow_self_names.insert(hir_name, self_name);
                    }
                    continue;
                }
                let native_method_value = self.native_instance_method_value(init).or_else(|| {
                    let Expr::Ident(identifier) = init else {
                        return None;
                    };
                    self.native_method_values
                        .get(&self.resolve_binding(identifier.sym.as_ref()))
                        .cloned()
                });
                let propagated_discriminants = self.expression_union_discriminants(init);
                let propagated_array_discriminants =
                    self.expression_array_element_discriminants(init);
                let propagated_nested_array_discriminants =
                    self.expression_nested_array_discriminants(init);
                let propagated_object_array_property_discriminants =
                    self.expression_object_array_property_discriminants(init);
                let propagated_object_function_property_discriminants =
                    self.expression_object_function_property_discriminants(init);
                let propagated_function_discriminants =
                    self.expression_function_discriminants(init);
                let propagated_function_array_discriminants =
                    self.expression_function_array_discriminants(init);
                let propagated_function_nested_array_discriminants =
                    self.expression_function_nested_array_discriminants(init);
                let propagated_function_object_array_property_discriminants =
                    self.expression_function_object_array_property_discriminants(init);
                let propagated_function_object_function_property_discriminants =
                    self.expression_function_object_function_property_discriminants(init);
                let value = match (init, annotated.as_ref()) {
                    (Expr::Arrow(arrow), Some(HirType::Function(params, ret))) => {
                        self.lower_contextual_arrow(arrow, params, Some(ret))?
                    }
                    (Expr::Arrow(arrow), Some(HirType::CallableFunction(params, _, rest, ret))) => {
                        let mut abi_params = params.clone();
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        self.lower_contextual_arrow(arrow, &abi_params, Some(ret))?
                    }
                    (Expr::Fn(function), Some(HirType::Function(params, ret))) => {
                        let arrow = function_expression_as_arrow(function)?;
                        self.lower_contextual_arrow(&arrow, params, Some(ret))?
                    }
                    (Expr::Fn(function), Some(HirType::CallableFunction(params, _, rest, ret))) => {
                        let arrow = function_expression_as_arrow(function)?;
                        let mut abi_params = params.clone();
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        self.lower_contextual_arrow(&arrow, &abi_params, Some(ret))?
                    }
                    (
                        Expr::Ident(identifier),
                        Some(HirType::Function(_, _) | HirType::CallableFunction(..)),
                    ) => {
                        let source = self.resolve_binding(identifier.sym.as_ref());
                        if self.scope.contains_key(&source) {
                            self.lower_expr(init)?
                        } else {
                            let signature = self.signatures.get(&source).ok_or_else(|| {
                                format!("unknown function value `{}`", identifier.sym)
                            })?;
                            if signature.is_extern || !signature.generic_type_params.is_empty() {
                                return Err(format!(
                                    "function value `{}` needs a monomorphic implementation",
                                    identifier.sym
                                ));
                            }
                            let ret = if signature.is_async {
                                HirType::Promise(Box::new(signature.ret.clone()))
                            } else {
                                signature.ret.clone()
                            };
                            HirExpr::FunctionRef(source, signature.params.clone(), ret)
                        }
                    }
                    _ => self.lower_expr(init)?,
                };

                let ty = match annotated {
                    Some(ty) => ty,
                    None => self.infer_expr_type(&value).map_err(|e| {
                        format!(
                            "cannot infer the type of `{name}`: {e} \
                             (add an explicit type annotation)"
                        )
                    })?,
                };
                let value = self.coerce_to_declared(&ty, value)?;

                let hir_name = self.bind_local(&name, ty.clone());
                if let Some(source) = correlated_alias_source {
                    self.propagate_destructured_union_alias(&source, &hir_name);
                }
                if let Some(annotation) = &binding.type_ann {
                    let discriminants =
                        object_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                    if !discriminants.is_empty() {
                        self.union_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    let element_discriminants = array_element_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !element_discriminants.is_empty() {
                        self.array_element_discriminants
                            .insert(hir_name.clone(), element_discriminants);
                    }
                    let nested_discriminants = nested_array_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !nested_discriminants.is_empty() {
                        self.nested_array_discriminants
                            .insert(hir_name.clone(), nested_discriminants);
                    }
                    let property_discriminants = object_array_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !property_discriminants.is_empty() {
                        self.object_array_property_discriminants
                            .insert(hir_name.clone(), property_discriminants);
                    }
                    let function_property_discriminants = object_function_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !function_property_discriminants.is_empty() {
                        self.object_function_property_discriminants
                            .insert(hir_name.clone(), function_property_discriminants);
                    }
                    let return_discriminants = function_return_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_discriminants.is_empty() {
                        self.function_value_discriminants
                            .insert(hir_name.clone(), return_discriminants);
                    }
                    let return_array_discriminants = function_return_array_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_array_discriminants.is_empty() {
                        self.function_value_array_discriminants
                            .insert(hir_name.clone(), return_array_discriminants);
                    }
                    let return_nested_discriminants = function_return_nested_array_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    if !return_nested_discriminants.is_empty() {
                        self.function_value_nested_array_discriminants
                            .insert(hir_name.clone(), return_nested_discriminants);
                    }
                    let return_property_discriminants =
                        function_return_object_array_property_discriminants(
                            &annotation.type_ann,
                            self.generic_interfaces,
                        );
                    if !return_property_discriminants.is_empty() {
                        self.function_value_object_array_property_discriminants
                            .insert(hir_name.clone(), return_property_discriminants);
                    }
                    let return_function_property_discriminants =
                        function_return_object_function_property_discriminants(
                            &annotation.type_ann,
                            self.generic_interfaces,
                        );
                    if !return_function_property_discriminants.is_empty() {
                        self.function_value_object_function_property_discriminants
                            .insert(hir_name.clone(), return_function_property_discriminants);
                    }
                } else if let Some(discriminants) = propagated_discriminants {
                    self.union_discriminants
                        .insert(hir_name.clone(), discriminants);
                }
                if binding.type_ann.is_none() {
                    if let Some(discriminants) = propagated_array_discriminants {
                        self.array_element_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_nested_array_discriminants {
                        self.nested_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_object_array_property_discriminants {
                        self.object_array_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_object_function_property_discriminants {
                        self.object_function_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                }
                if binding.type_ann.is_none() {
                    if let Some(discriminants) = propagated_function_discriminants {
                        self.function_value_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_function_array_discriminants {
                        self.function_value_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) = propagated_function_nested_array_discriminants {
                        self.function_value_nested_array_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) =
                        propagated_function_object_array_property_discriminants
                    {
                        self.function_value_object_array_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                    if let Some(discriminants) =
                        propagated_function_object_function_property_discriminants
                    {
                        self.function_value_object_function_property_discriminants
                            .insert(hir_name.clone(), discriminants);
                    }
                }
                if let Some(method) = native_method_value {
                    self.native_method_values.insert(hir_name.clone(), method);
                }
                statements.push(HirStmt::Let(hir_name, ty, value));
                continue;
            }

            let init = decl
                .init
                .as_deref()
                .ok_or("destructuring declarations need an initializer")?;
            let propagated_discriminants = self.expression_union_discriminants(init);
            let propagated_property_discriminants =
                self.expression_object_array_property_discriminants(init);
            let propagated_function_property_discriminants =
                self.expression_object_function_property_discriminants(init);
            let mut value = self.lower_expr(init)?;
            let annotation = match &decl.name {
                Pat::Array(pattern) => pattern.type_ann.as_ref(),
                Pat::Object(pattern) => pattern.type_ann.as_ref(),
                _ => None,
            };
            let ty = if let Some(annotation) = annotation {
                let ty = lower_ts_type(
                    &annotation.type_ann,
                    self.interfaces,
                    self.generic_interfaces,
                )?;
                value = self.coerce_to_declared(&ty, value)?;
                ty
            } else if matches!(&decl.name, Pat::Array(_)) {
                if let HirExpr::ArrayLit(elements) = &value {
                    HirType::Tuple(
                        elements
                            .iter()
                            .map(|element| self.infer_expr_type(element))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else {
                    self.infer_expr_type(&value)?
                }
            } else {
                self.infer_expr_type(&value)?
            };
            let destructurable_union = matches!(
                &ty,
                HirType::Union(elements)
                    if matches!(&decl.name, Pat::Object(_))
                        && elements.iter().all(|element| matches!(element, HirType::Object(_)))
            );
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_)) && !destructurable_union {
                return Err(format!(
                    "destructuring requires a fixed-shape object or tuple, got {ty:?}"
                ));
            }
            let temporary = format!("__thaw_destructure_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            let declared_discriminants = annotation.and_then(|annotation| {
                let discriminants =
                    object_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) = declared_discriminants.or(propagated_discriminants) {
                self.union_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            let declared_property_discriminants = annotation.and_then(|annotation| {
                let discriminants = object_array_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) =
                declared_property_discriminants.or(propagated_property_discriminants)
            {
                self.object_array_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            let declared_function_property_discriminants = annotation.and_then(|annotation| {
                let discriminants = object_function_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!discriminants.is_empty()).then_some(discriminants)
            });
            if let Some(discriminants) = declared_function_property_discriminants
                .or(propagated_function_property_discriminants)
            {
                self.object_function_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            statements.push(HirStmt::Let(temporary.clone(), ty.clone(), value));
            self.lower_binding_pattern(&decl.name, HirExpr::Var(temporary), &ty, &mut statements)?;
        }
        Ok(statements)
    }
}
