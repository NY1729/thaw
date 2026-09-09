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
        let saved_json_narrowings = self.json_narrowings.clone();
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
        self.json_narrowings = saved_json_narrowings;
        self.union_narrowings = saved_union_narrowings;
        self.generic_arrows = saved_generic_arrows;
        self.generic_arrow_self_names = saved_generic_arrow_self_names;
        self.generic_named_templates = saved_generic_named_templates;
        self.native_method_values = saved_native_method_values;
        lowered
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        #[derive(Default)]
        struct BindingConsumers {
            members: HashSet<Symbol>,
            awaited: HashSet<Symbol>,
        }
        impl Visit for BindingConsumers {
            fn visit_member_expr(&mut self, member: &MemberExpr) {
                if let Expr::Ident(receiver) = member.obj.as_ref() {
                    self.members.insert(receiver.sym.to_string());
                }
                member.visit_children_with(self);
            }

            fn visit_await_expr(&mut self, await_expr: &AwaitExpr) {
                if let Expr::Ident(value) = await_expr.arg.as_ref() {
                    self.awaited.insert(value.sym.to_string());
                }
                await_expr.visit_children_with(self);
            }
        }
        let mut consumers = BindingConsumers::default();
        for stmt in stmts {
            stmt.visit_with(&mut consumers);
        }
        self.member_receiver_bindings.extend(consumers.members);
        self.awaited_bindings.extend(consumers.awaited);

        let mut out = Vec::new();
        for stmt in stmts {
            out.extend(self.lower_stmt_seq(stmt)?);
            if let Stmt::Expr(expression) = stmt {
                if let Some(targets) = self.assertion_union_narrowing(&expression.expr) {
                    for target in targets {
                        self.union_narrowings
                            .insert(target.name, (target.matching, target.elements));
                    }
                }
            }
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

    fn infer_generator_types(
        &self,
        body: &[HirStmt],
        yields: &str,
        returns: &str,
    ) -> Result<(HirType, HirType), String> {
        fn collect<'a>(
            statements: &'a [HirStmt],
            yields: &str,
            returns: &str,
            yielded: &mut Vec<&'a HirExpr>,
            returned: &mut Vec<&'a HirExpr>,
        ) {
            for statement in statements {
                match statement {
                    HirStmt::Expr(HirExpr::Assign(channel, value)) if channel == yields => {
                        if matches!(value.as_ref(), HirExpr::ArrayConcat(..)) {
                            yielded.push(value);
                        }
                    }
                    HirStmt::Expr(HirExpr::Call(callee, arguments))
                        if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_array_push") =>
                    {
                        if let [HirExpr::Var(channel), value] = arguments.as_slice() {
                            if channel == yields {
                                yielded.push(value);
                            } else if channel == returns {
                                returned.push(value);
                            }
                        }
                    }
                    HirStmt::If(_, then_body, else_body) => {
                        collect(then_body, yields, returns, yielded, returned);
                        collect(else_body, yields, returns, yielded, returned);
                    }
                    HirStmt::While(_, loop_body) => {
                        collect(loop_body, yields, returns, yielded, returned)
                    }
                    HirStmt::Try(try_body, _, catch_body) => {
                        collect(try_body, yields, returns, yielded, returned);
                        collect(catch_body, yields, returns, yielded, returned);
                    }
                    _ => {}
                }
            }
        }

        let mut yielded = Vec::new();
        let mut returned = Vec::new();
        collect(body, yields, returns, &mut yielded, &mut returned);
        let infer = |values: Vec<&HirExpr>| -> Result<HirType, String> {
            let mut types = Vec::new();
            for value in values {
                let ty = match value {
                    HirExpr::ArrayConcat(_, element) => element.clone(),
                    _ => self.infer_expr_type(value)?,
                };
                if ty != HirType::Dynamic && !types.contains(&ty) {
                    types.push(ty);
                }
            }
            Ok(match types.as_slice() {
                [] => HirType::Undefined,
                [ty] => ty.clone(),
                _ => HirType::Union(types),
            })
        };
        Ok((infer(yielded)?, infer(returned)?))
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
        if let Some((name, predicate)) = self.predicate_call_target(expr) {
            if matches!(self.scope.get(&name), Some(HirType::Optional(payload)) if payload.as_ref() == &predicate)
            {
                return Some((name, predicate, true, 0));
            }
        }
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

    fn json_typeof_narrowing(&self, expr: &Expr) -> Option<(Symbol, HirType, bool)> {
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
        let Expr::Ident(target) = target else {
            return None;
        };
        let name = self.resolve_binding(target.sym.as_ref());
        if self.scope.get(&name) != Some(&HirType::Json) {
            return None;
        }
        let narrowed = match type_name.as_ref() {
            "string" => HirType::Str,
            "number" => HirType::F64,
            "boolean" => HirType::Bool,
            _ => return None,
        };
        Some((name, narrowed, equal_when_true))
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

    fn union_boolean_discriminant_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let Expr::Member(member) = expr else {
            return None;
        };
        let Expr::Ident(identifier) = member.obj.as_ref() else {
            return None;
        };
        let property = member_property_name(&member.prop)?;
        let name = self.resolve_binding(identifier.sym.as_ref());
        let HirType::Union(elements) = self.scope.get(&name)? else {
            return None;
        };
        let values = self.union_discriminants.get(&name)?.get(&property)?;
        let allowed = self
            .union_narrowings
            .get(&name)
            .map(|(allowed, _)| allowed.clone())
            .unwrap_or_else(|| (0..elements.len()).collect());
        let matching = allowed
            .iter()
            .copied()
            .filter(|index| values[*index] == Some(HirLit::Bool(true)))
            .collect::<Vec<_>>();
        (!matching.is_empty()).then(|| {
            (
                vec![UnionNarrowingTarget {
                    name,
                    matching,
                    allowed,
                    elements: elements.clone(),
                }],
                true,
                true,
            )
        })
    }

    fn union_instanceof_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let Expr::Bin(binary) = expr else {
            return None;
        };
        if binary.op != BinaryOp::InstanceOf {
            return None;
        }
        let (Expr::Ident(value), Expr::Ident(class)) =
            (binary.left.as_ref(), binary.right.as_ref())
        else {
            return None;
        };
        let name = self.resolve_binding(value.sym.as_ref());
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
            .filter(|index| class_type_has_identity(&elements[*index], class.sym.as_ref()))
            .collect::<Vec<_>>();
        (!matching.is_empty()).then(|| {
            (
                vec![UnionNarrowingTarget {
                    name,
                    matching,
                    allowed,
                    elements: elements.clone(),
                }],
                true,
                true,
            )
        })
    }

    fn union_in_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let Expr::Bin(binary) = expr else { return None };
        if binary.op != BinaryOp::In {
            return None;
        }
        let (Expr::Lit(Lit::Str(property)), Expr::Ident(value)) =
            (binary.left.as_ref(), binary.right.as_ref())
        else {
            return None;
        };
        let property = property.value.to_string_lossy();
        let name = self.resolve_binding(value.sym.as_ref());
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
                matches!(&elements[*index], HirType::Object(fields) if fields.iter().any(|(name, _)| name == property.as_ref()))
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
                true,
                true,
            )
        })
    }

    fn predicate_call_target(&self, expr: &Expr) -> Option<(Symbol, HirType)> {
        let Expr::Call(call) = expr else { return None };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Ident(callee) = callee.as_ref() else {
            return None;
        };
        let signature = self.signatures.get(callee.sym.as_ref())?;
        let (parameter, predicate, _) = signature.type_predicate.as_ref()?;
        let argument = call.args.get(*parameter)?;
        if argument.spread.is_some() {
            return None;
        }
        let Expr::Ident(argument) = argument.expr.as_ref() else {
            return None;
        };
        let name = self.resolve_binding(argument.sym.as_ref());
        let mut inferred = HashMap::new();
        for (pattern, argument) in signature.generic_param_patterns.iter().zip(&call.args) {
            let actual = match argument.expr.as_ref() {
                Expr::Ident(argument) => self
                    .scope
                    .get(&self.resolve_binding(argument.sym.as_ref()))?
                    .clone(),
                Expr::Lit(Lit::Num(_)) => HirType::F64,
                Expr::Lit(Lit::Str(_)) => HirType::Str,
                Expr::Lit(Lit::Bool(_)) => HirType::Bool,
                _ => continue,
            };
            match_generic_pattern(pattern, &actual, &mut inferred).ok()?;
        }
        let predicate = instantiate_generic_pattern(predicate, &inferred).ok()?;
        Some((name, predicate))
    }

    fn union_predicate_call_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let (name, predicate) = self.predicate_call_target(expr)?;
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
            .filter(|index| type_satisfies_constraint(&elements[*index], &predicate))
            .collect::<Vec<_>>();
        (!matching.is_empty()).then(|| {
            (
                vec![UnionNarrowingTarget {
                    name,
                    matching,
                    allowed,
                    elements: elements.clone(),
                }],
                true,
                true,
            )
        })
    }

    fn assertion_union_narrowing(&self, expr: &Expr) -> Option<Vec<UnionNarrowingTarget>> {
        let Expr::Call(call) = expr else { return None };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Ident(callee) = callee.as_ref() else {
            return None;
        };
        if !self
            .signatures
            .get(callee.sym.as_ref())?
            .type_predicate
            .as_ref()?
            .2
        {
            return None;
        }
        self.union_predicate_call_narrowing(expr)
            .map(|(targets, _, _)| targets)
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
                if let Some((right_targets, right_equal, right_complement)) =
                    self.union_narrowing(&binary.right)
                {
                    let left_truth: Vec<_> = branch_targets(true);
                    let right_truth = |target: &UnionNarrowingTarget| {
                        if right_equal {
                            target.matching.clone()
                        } else if right_complement {
                            target
                                .allowed
                                .iter()
                                .filter(|index| !target.matching.contains(index))
                                .copied()
                                .collect()
                        } else {
                            target.allowed.clone()
                        }
                    };
                    if left_truth.len() == right_targets.len()
                        && left_truth.iter().zip(&right_targets).all(|(left, right)| {
                            left.name == right.name
                                && left.allowed == right.allowed
                                && left.elements == right.elements
                        })
                    {
                        let combined = left_truth
                            .into_iter()
                            .zip(&right_targets)
                            .map(|(mut left, right)| {
                                let right = right_truth(right);
                                if binary.op == BinaryOp::LogicalAnd {
                                    left.matching.retain(|index| right.contains(index));
                                } else {
                                    for index in right {
                                        if !left.matching.contains(&index) {
                                            left.matching.push(index);
                                        }
                                    }
                                }
                                left
                            })
                            .collect();
                        return Some((combined, true, true));
                    }
                }
                return if binary.op == BinaryOp::LogicalAnd {
                    Some((branch_targets(true), true, false))
                } else {
                    Some((branch_targets(false), false, false))
                };
            }
        }
        self.union_typeof_narrowing(expr)
            .or_else(|| self.union_predicate_call_narrowing(expr))
            .or_else(|| self.union_member_equality_narrowing(expr))
            .or_else(|| self.union_boolean_discriminant_narrowing(expr))
            .or_else(|| self.union_instanceof_narrowing(expr))
            .or_else(|| self.union_in_narrowing(expr))
    }

    fn lower_body_with_union_narrowing(
        &mut self,
        stmt: &Stmt,
        narrowing: Option<&[UnionNarrowingTarget]>,
        optional: Option<&(Symbol, HirType, u8)>,
        json: Option<&(Symbol, HirType)>,
    ) -> Result<Vec<HirStmt>, String> {
        let saved = self.union_narrowings.clone();
        let saved_json = self.json_narrowings.clone();
        if let Some(targets) = narrowing {
            for target in targets {
                self.union_narrowings.insert(
                    target.name.clone(),
                    (target.matching.clone(), target.elements.clone()),
                );
            }
        }
        if let Some((name, ty)) = json {
            self.json_narrowings.insert(name.clone(), ty.clone());
        }
        let lowered = self.lower_body_with_optional_narrowing(stmt, optional);
        self.union_narrowings = saved;
        self.json_narrowings = saved_json;
        lowered
    }

}
