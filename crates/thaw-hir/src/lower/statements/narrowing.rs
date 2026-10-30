fn collect_return_values<'a>(stmts: &'a [HirStmt], out: &mut Vec<&'a HirExpr>, bare: &mut bool) {
    for stmt in stmts {
        match stmt {
            HirStmt::Return(Some(value)) => out.push(value),
            HirStmt::Return(None) => *bare = true,
            HirStmt::If(_, then_body, else_body) => {
                collect_return_values(then_body, out, bare);
                collect_return_values(else_body, out, bare);
            }
            HirStmt::While(_, body) => collect_return_values(body, out, bare),
            HirStmt::Finally(body, _) => collect_return_values(body, out, bare),
            HirStmt::Try(body, _, catch_body, _) => {
                collect_return_values(body, out, bare);
                collect_return_values(catch_body, out, bare);
            }
            _ => {}
        }
    }
}


impl<'a> FnLowerer<'a> {
    fn save_narrowings(&self) -> NarrowingSnapshot {
        NarrowingSnapshot {
            optional: self.narrowings.clone(),
            nullable: self.nullable_narrowings.clone(),
            nullish: self.nullish_narrowings.clone(),
            partial_nullable: self.partial_nullable_narrowings.clone(),
            write_versions: self.narrowing_write_versions.clone(),
        }
    }

    fn restore_narrowings_for_sibling(&mut self, saved: &NarrowingSnapshot) {
        self.narrowings = saved.optional.clone();
        self.nullable_narrowings = saved.nullable.clone();
        self.nullish_narrowings = saved.nullish.clone();
        self.partial_nullable_narrowings = saved.partial_nullable.clone();
    }

    fn restore_narrowings(&mut self, saved: NarrowingSnapshot) {
        let versions = &self.narrowing_write_versions;
        let unchanged = |name: &Symbol| versions.get(name) == saved.write_versions.get(name);
        self.narrowings = saved.optional.into_iter().filter(|(name, _)| unchanged(name)).collect();
        self.nullable_narrowings = saved.nullable.into_iter().filter(|(name, _)| unchanged(name)).collect();
        self.nullish_narrowings = saved.nullish.into_iter().filter(|(name, _)| unchanged(name)).collect();
        self.partial_nullable_narrowings = saved.partial_nullable.into_iter().filter(|(name, _)| unchanged(name)).collect();
    }

    fn record_binding_write(&mut self, name: &str) {
        self.narrowings.remove(name);
        self.nullable_narrowings.remove(name);
        self.nullish_narrowings.remove(name);
        self.partial_nullable_narrowings.remove(name);
        self.json_narrowings.remove(name);
        *self.narrowing_write_versions.entry(name.to_string()).or_default() += 1;
    }

    fn bind_local(&mut self, source_name: &str, ty: HirType) -> Symbol {
        let hir_name = if self.scope.contains_key(source_name)
            || self.signatures.contains_key(source_name)
            || self.used_hir_bindings.contains(source_name)
        {
            loop {
                let name = format!("{source_name}__thaw_local_{}", self.next_binding);
                self.next_binding += 1;
                if !self.scope.contains_key(&name)
                    && !self.signatures.contains_key(&name)
                    && !self.used_hir_bindings.contains(&name)
                {
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

    fn bind_decl_local(&mut self, source_name: &str, ty: HirType, immutable: bool) -> Symbol {
        let hir_name = self.bind_local(source_name, ty);
        if immutable {
            self.immutable_bindings.insert(hir_name.clone());
        }
        hir_name
    }

    // The name marks a binding whose cell must be replaced on every for-loop
    // iteration, including when an async function resumes in a new frame call.
    fn bind_for_iteration_local(&mut self, source_name: &str, ty: HirType) -> Symbol {
        let outer_name = self.resolve_binding(source_name);
        let immutable = self.immutable_bindings.contains(&outer_name);
        let hir_name = format!("@@thaw_for_iteration_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(hir_name.clone(), ty);
        self.used_hir_bindings.insert(hir_name.clone());
        self.bindings
            .entry(source_name.to_string())
            .or_default()
            .push(hir_name.clone());
        if immutable {
            self.immutable_bindings.insert(hir_name.clone());
        }
        hir_name
    }

    fn lower_scoped_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let saved = self.bindings.clone();
        let saved_narrowings = self.save_narrowings();
        let saved_json_narrowings = self.json_narrowings.clone();
        let saved_union_narrowings = self.union_narrowings.clone();
        let saved_generic_arrows = self.generic_arrows.clone();
        let saved_generic_non_arrow_names = self.generic_non_arrow_names.clone();
        let saved_generic_non_arrow_receivers = self.generic_non_arrow_receivers.clone();
        let saved_generic_non_arrow_receiver_templates =
            self.generic_non_arrow_receiver_templates.clone();
        let saved_generic_arrow_self_names = self.generic_arrow_self_names.clone();
        let saved_generic_named_templates = self.generic_named_templates.clone();
        let saved_deferred_generic_binds = self.deferred_generic_binds.clone();
        let saved_native_method_values = self.native_method_values.clone();
        let lowered = self.lower_stmts(stmts);
        self.bindings = saved;
        self.restore_narrowings(saved_narrowings);
        self.json_narrowings = saved_json_narrowings;
        self.union_narrowings = saved_union_narrowings;
        self.generic_arrows = saved_generic_arrows;
        self.generic_non_arrow_names = saved_generic_non_arrow_names;
        self.generic_non_arrow_receivers = saved_generic_non_arrow_receivers;
        self.generic_non_arrow_receiver_templates = saved_generic_non_arrow_receiver_templates;
        self.generic_arrow_self_names = saved_generic_arrow_self_names;
        self.generic_named_templates = saved_generic_named_templates;
        self.deferred_generic_binds = saved_deferred_generic_binds;
        self.native_method_values = saved_native_method_values;
        lowered
    }

    /// JavaScript hoists a block's function declarations to its start. A
    /// declaration is moved up only when it names no `let`/`const`/`class`/
    /// `var` of the same block (those cells do not exist yet), and
    /// declarations are ordered so each one follows the siblings it calls.
    fn hoist_function_declarations(stmts: &[Stmt]) -> std::borrow::Cow<'_, [Stmt]> {
        use swc_ecma_visit::{Visit, VisitWith};
        #[derive(Default)]
        struct Names(HashSet<String>);
        impl Visit for Names {
            fn visit_ident(&mut self, ident: &swc_ecma_ast::Ident) {
                self.0.insert(ident.sym.to_string());
            }
        }
        let function_count = stmts
            .iter()
            .filter(|stmt| matches!(stmt, Stmt::Decl(Decl::Fn(_))))
            .count();
        if function_count == 0 {
            return std::borrow::Cow::Borrowed(stmts);
        }
        let mut block_cells = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::Decl(Decl::Var(var)) => {
                    for declarator in &var.decls {
                        Self::collect_pattern_bindings(&declarator.name, &mut block_cells);
                    }
                }
                Stmt::Decl(Decl::Using(using)) => {
                    for declarator in &using.decls {
                        Self::collect_pattern_bindings(&declarator.name, &mut block_cells);
                    }
                }
                Stmt::Decl(Decl::Class(class)) => block_cells.push(class.ident.sym.to_string()),
                _ => {}
            }
        }
        let mut functions: Vec<(usize, String, HashSet<String>)> = Vec::new();
        for (index, stmt) in stmts.iter().enumerate() {
            let Stmt::Decl(Decl::Fn(function)) = stmt else { continue };
            let mut names = Names::default();
            function.function.visit_with(&mut names);
            if block_cells.iter().any(|cell| names.0.contains(cell)) {
                continue;
            }
            functions.push((index, function.ident.sym.to_string(), names.0));
        }
        if functions.is_empty() {
            return std::borrow::Cow::Borrowed(stmts);
        }
        // Dependencies first; a cycle keeps source order for the rest.
        let sibling_names: HashSet<String> =
            functions.iter().map(|(_, name, _)| name.clone()).collect();
        let mut ordered: Vec<usize> = Vec::new();
        let mut placed: HashSet<String> = HashSet::new();
        let mut remaining: Vec<usize> = (0..functions.len()).collect();
        while !remaining.is_empty() {
            let ready = remaining.iter().position(|&candidate| {
                let (_, name, references) = &functions[candidate];
                references.iter().all(|reference| {
                    reference == name
                        || !sibling_names.contains(reference)
                        || placed.contains(reference)
                })
            });
            let next = ready.unwrap_or(0);
            let candidate = remaining.remove(next);
            placed.insert(functions[candidate].1.clone());
            ordered.push(candidate);
        }
        let hoisted: HashSet<usize> = functions.iter().map(|(index, _, _)| *index).collect();
        let mut result: Vec<Stmt> = ordered
            .into_iter()
            .map(|candidate| stmts[functions[candidate].0].clone())
            .collect();
        result.extend(
            stmts
                .iter()
                .enumerate()
                .filter(|(index, _)| !hoisted.contains(index))
                .map(|(_, stmt)| stmt.clone()),
        );
        std::borrow::Cow::Owned(result)
    }

    /// `const even = (n: number): boolean => .. odd(n - 1); const odd = ..`:
    /// a fully annotated const arrow that is referenced by itself or by an
    /// earlier arrow gets a `let name = <throwing stub>` placed before the
    /// first referencing declaration, and its declaration becomes `name = arrow`.
    fn forward_declare_arrows(stmts: &[Stmt]) -> std::borrow::Cow<'_, [Stmt]> {
        use swc_ecma_ast::{ArrowExpr, ArrowFunctionBody, FunctionBody, ThrowStmt};
        // (statement index, name, arrow) for annotated single const arrows.
        let mut arrows: Vec<(usize, Symbol, &ArrowExpr)> = Vec::new();
        for (index, stmt) in stmts.iter().enumerate() {
            let Stmt::Decl(Decl::Var(decl)) = stmt else { continue };
            let [declarator] = decl.decls.as_slice() else { continue };
            let (Pat::Ident(binding), Some(init)) = (&declarator.name, declarator.init.as_deref()) else { continue };
            let Expr::Arrow(arrow) = init else { continue };
            if decl.kind == VarDeclKind::Const
                && binding.type_ann.is_none()
                && arrow.type_params.is_none()
                && !arrow.is_generator
                && arrow.return_type.is_some()
                && arrow.params.iter().all(|p| matches!(p, Pat::Ident(b) if b.type_ann.is_some() && !b.id.optional))
            {
                arrows.push((index, binding.id.sym.to_string(), arrow));
            }
        }
        struct Refs(HashSet<Symbol>);
        impl Visit for Refs {
            fn visit_ident(&mut self, ident: &swc_ecma_ast::Ident) {
                self.0.insert(ident.sym.to_string());
            }
        }
        // name -> earliest arrow position (in `arrows`) that needs it declared.
        let mut needed: Vec<(Symbol, usize)> = Vec::new();
        for (position, (_, _, arrow)) in arrows.iter().enumerate() {
            let mut refs = Refs(HashSet::new());
            arrow.body.visit_with(&mut refs);
            for (other, (_, name, _)) in arrows.iter().enumerate() {
                if other >= position
                    && refs.0.contains(name)
                    && !needed.iter().any(|(n, _)| n == name)
                {
                    needed.push((name.clone(), position));
                }
            }
        }
        if needed.is_empty() {
            return std::borrow::Cow::Borrowed(stmts);
        }
        let sp = swc_common::DUMMY_SP;
        let stub = |arrow: &ArrowExpr| {
            let mut stub = arrow.clone();
            stub.is_async = false;
            stub.body = Box::new(ArrowFunctionBody::FunctionBody(FunctionBody {
                span: sp,
                stmts: vec![Stmt::Throw(ThrowStmt {
                    span: sp,
                    arg: Box::new(Expr::Lit(Lit::Num(swc_ecma_ast::Number { span: sp, value: 0.0, raw: None }))),
                })],
            }));
            // An async arrow keeps its annotated `Promise<T>` return type.
            stub
        };
        let mut result = Vec::with_capacity(stmts.len() + needed.len());
        for (index, stmt) in stmts.iter().enumerate() {
            let Some(position) = arrows.iter().position(|(i, _, _)| *i == index) else {
                result.push(stmt.clone());
                continue;
            };
            let (_, name, arrow) = &arrows[position];
            for (n, first) in &needed {
                if *first == position {
                    let (_, _, a) = arrows.iter().find(|(_, an, _)| an == n).unwrap();
                    result.push(Stmt::Decl(Decl::Var(Box::new(VarDecl {
                        span: sp,
                        ctxt: swc_common::SyntaxContext::empty(),
                        kind: VarDeclKind::Let,
                        declare: false,
                        decls: vec![swc_ecma_ast::VarDeclarator {
                            span: sp,
                            name: Pat::Ident(swc_ecma_ast::BindingIdent {
                                id: swc_ecma_ast::Ident::new_no_ctxt(n.as_str().into(), sp),
                                type_ann: None,
                            }),
                            init: Some(Box::new(Expr::Arrow(stub(a)))),
                            definite: false,
                        }],
                    }))));
                }
            }
            if needed.iter().any(|(n, _)| n == name) {
                result.push(Stmt::Expr(swc_ecma_ast::ExprStmt {
                    span: sp,
                    expr: Box::new(Expr::Assign(swc_ecma_ast::AssignExpr {
                        span: sp,
                        op: swc_ecma_ast::AssignOp::Assign,
                        left: swc_ecma_ast::AssignTarget::Simple(swc_ecma_ast::SimpleAssignTarget::Ident(
                            swc_ecma_ast::BindingIdent {
                                id: swc_ecma_ast::Ident::new_no_ctxt(name.as_str().into(), sp),
                                type_ann: None,
                            },
                        )),
                        right: Box::new(Expr::Arrow((*arrow).clone())),
                    })),
                }));
            } else {
                result.push(stmt.clone());
            }
        }
        std::borrow::Cow::Owned(result)
    }

    /// A closure that mentions a `const f = <arrow/function>` declared later in
    /// the same block is fine in JS when called after the declaration runs.
    /// Creating a closure has no side effects, so move such a declaration up
    /// to just before its first mention, unless it captures a block binding
    /// declared at or after that point (mutual recursion stays in order).
    fn hoist_forward_closure_consts(stmts: &[Stmt]) -> std::borrow::Cow<'_, [Stmt]> {
        use swc_ecma_visit::{Visit, VisitWith};
        #[derive(Default)]
        struct Names(HashSet<String>);
        impl Visit for Names {
            fn visit_ident(&mut self, ident: &swc_ecma_ast::Ident) {
                self.0.insert(ident.sym.to_string());
            }
        }
        let closure_name = |stmt: &Stmt| -> Option<String> {
            let Stmt::Decl(Decl::Var(var)) = stmt else { return None };
            let [declarator] = var.decls.as_slice() else { return None };
            let (Pat::Ident(binding), Some(init)) = (&declarator.name, &declarator.init) else {
                return None;
            };
            (var.kind == VarDeclKind::Const
                && matches!(init.as_ref(), Expr::Arrow(_) | Expr::Fn(_)))
            .then(|| binding.id.sym.to_string())
        };
        if !stmts.iter().any(|stmt| closure_name(stmt).is_some()) {
            return std::borrow::Cow::Borrowed(stmts);
        }
        let mut order: Vec<Stmt> = stmts.to_vec();
        let mut moved = false;
        for _ in 0..order.len() {
            let mentions: Vec<HashSet<String>> = order
                .iter()
                .map(|stmt| {
                    let mut names = Names::default();
                    stmt.visit_with(&mut names);
                    names.0
                })
                .collect();
            let mut cells: HashMap<String, usize> = HashMap::new();
            for (index, stmt) in order.iter().enumerate() {
                let mut bound = Vec::new();
                match stmt {
                    Stmt::Decl(Decl::Var(var)) => {
                        for declarator in &var.decls {
                            Self::collect_pattern_bindings(&declarator.name, &mut bound);
                        }
                    }
                    Stmt::Decl(Decl::Class(class)) => bound.push(class.ident.sym.to_string()),
                    Stmt::Decl(Decl::Fn(function)) => bound.push(function.ident.sym.to_string()),
                    _ => {}
                }
                for name in bound {
                    cells.insert(name, index);
                }
            }
            let step = (0..order.len()).find_map(|j| {
                let name = closure_name(&order[j])?;
                let i = (0..j).find(|&i| mentions[i].contains(&name))?;
                mentions[j]
                    .iter()
                    .all(|mention| {
                        *mention == name
                            || cells.get(mention).is_none_or(|&declared| declared < i)
                    })
                    .then_some((i, j))
            });
            let Some((i, j)) = step else { break };
            let stmt = order.remove(j);
            order.insert(i, stmt);
            moved = true;
        }
        if moved {
            std::borrow::Cow::Owned(order)
        } else {
            std::borrow::Cow::Borrowed(stmts)
        }
    }

    /// `const u = new Uint8Array(n)` becomes a native byte store (O(1) element access, no JS realm)
    /// when `u` is only ever indexed (`u[i]`, `u[i] = v`) or measured (`u.length`) in `later`;
    /// any other use keeps the live typed array, which supports the full API.
    fn mark_native_uint8array_declarations(&mut self, declaration: &VarDecl, later: &[Stmt]) {
        use swc_ecma_visit::{Visit, VisitWith};
        struct OnlyIndexed<'a> {
            name: &'a str,
            ok: bool,
        }
        impl Visit for OnlyIndexed<'_> {
            fn visit_ident(&mut self, ident: &swc_ecma_ast::Ident) {
                self.ok &= ident.sym != self.name;
            }
            fn visit_member_expr(&mut self, member: &swc_ecma_ast::MemberExpr) {
                let own = matches!(member.obj.as_ref(), Expr::Ident(id) if id.sym == self.name);
                let allowed = match &member.prop {
                    swc_ecma_ast::MemberProp::Computed(_) => true,
                    swc_ecma_ast::MemberProp::Ident(prop) => prop.sym == "length",
                    _ => false,
                };
                if own && allowed {
                    member.prop.visit_with(self);
                } else {
                    member.visit_children_with(self);
                }
            }
            // `u.length = n` is not an index write.
            fn visit_assign_expr(&mut self, assign: &swc_ecma_ast::AssignExpr) {
                if let Some(swc_ecma_ast::SimpleAssignTarget::Member(member)) = assign.left.as_simple() {
                    if matches!(member.obj.as_ref(), Expr::Ident(id) if id.sym == self.name)
                        && !matches!(member.prop, swc_ecma_ast::MemberProp::Computed(_))
                    {
                        self.ok = false;
                    }
                }
                assign.visit_children_with(self);
            }
        }
        if declaration.kind != VarDeclKind::Const {
            return;
        }
        for declarator in &declaration.decls {
            let (Pat::Ident(binding), Some(init)) = (&declarator.name, declarator.init.as_deref()) else {
                continue;
            };
            let Expr::New(construction) = init else { continue };
            let Some(arguments) = construction.args.as_deref() else { continue };
            if arguments.len() != 1
                || !construction.callee.as_ident().is_some_and(|class| class.sym == "Uint8Array")
            {
                continue;
            }
            let mut uses = OnlyIndexed { name: binding.id.sym.as_ref(), ok: true };
            for statement in later {
                statement.visit_with(&mut uses);
            }
            if uses.ok {
                self.native_u8_args.insert(arguments.as_ptr() as usize);
            }
        }
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let hoisted_vars = pre_read_var_names(stmts)
            .into_iter()
            .filter(|name| self.bindings.get(name).is_none_or(Vec::is_empty))
            .collect::<Vec<_>>();
        let var_hoisted;
        let stmts = if hoisted_vars.is_empty() {
            stmts
        } else {
            var_hoisted = hoist_var_names(stmts, &hoisted_vars);
            &var_hoisted[..]
        };
        let saved_tdz = self.tdz_pending.clone();
        let lowered = self.lower_stmts_in_order(stmts);
        self.tdz_pending = saved_tdz;
        lowered
    }

    fn lower_stmts_in_order(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let hoisted_closures = Self::hoist_forward_closure_consts(stmts);
        let forward = Self::forward_declare_arrows(&hoisted_closures);
        let hoisted = Self::hoist_function_declarations(&forward);
        let stmts: &[Stmt] = &hoisted;
        // Straight-line TDZ: a `let`/`const` read before its declaration statement.
        let mut tdz_declared: Vec<Vec<String>> = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            let mut names = Vec::new();
            if let Stmt::Decl(Decl::Var(declaration)) = stmt {
                if declaration.kind != VarDeclKind::Var {
                    for declarator in &declaration.decls {
                        let Pat::Ident(binding) = &declarator.name else { continue };
                        let name = binding.id.sym.to_string();
                        // Only a placeholder for the throwing read's static type.
                        let placeholder = match declarator.init.as_deref() {
                            Some(Expr::Lit(Lit::Num(_))) => HirLit::F64(0.0),
                            Some(Expr::Lit(Lit::Str(_))) => HirLit::Str(String::new()),
                            Some(Expr::Lit(Lit::Bool(_))) => HirLit::Bool(false),
                            _ => HirLit::Undefined,
                        };
                        let current = self.resolve_binding(&name);
                        self.tdz_pending.insert(name.clone(), (current, placeholder));
                        names.push(name);
                    }
                }
            }
            tdz_declared.push(names);
        }
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
        let mut using_disposals: Vec<HirStmt> = Vec::new();
        let mut using_flags = Vec::new();
        // `out` index from which each resource is live.
        let mut using_starts: Vec<usize> = Vec::new();
        for (index, stmt) in stmts.iter().enumerate() {
            if index > 0 {
                for name in &tdz_declared[index - 1] {
                    self.tdz_pending.remove(name);
                }
            }
            if let Stmt::Decl(Decl::Var(var_decl)) = stmt {
                self.mark_native_uint8array_declarations(var_decl, &stmts[index + 1..]);
                let uninit: Vec<Symbol> = var_decl
                    .decls
                    .iter()
                    .filter(|d| d.init.is_none())
                    .filter_map(|d| match &d.name {
                        Pat::Ident(b) => Some(b.id.sym.to_string()),
                        _ => None,
                    })
                    .collect();
                if !uninit.is_empty() {
                    struct Writes(HashSet<Symbol>);
                    impl Visit for Writes {
                        fn visit_assign_expr(&mut self, assign: &swc_ecma_ast::AssignExpr) {
                            if let Some(id) = assign.left.as_ident() {
                                self.0.insert(id.sym.to_string());
                            }
                            assign.visit_children_with(self);
                        }
                        fn visit_update_expr(&mut self, update: &swc_ecma_ast::UpdateExpr) {
                            if let Expr::Ident(id) = update.arg.as_ref() {
                                self.0.insert(id.sym.to_string());
                            }
                            update.visit_children_with(self);
                        }
                    }
                    let mut writes = Writes(HashSet::new());
                    for later in &stmts[index + 1..] {
                        later.visit_with(&mut writes);
                    }
                    self.assigned_later
                        .extend(uninit.into_iter().filter(|n| writes.0.contains(n)));
                }
            }
            if let Stmt::Decl(Decl::Using(using_decl)) = stmt {
                // Register each successful acquisition before the next initializer.
                for declarator in &using_decl.decls {
                    let mut single = using_decl.clone();
                    single.decls = vec![declarator.clone()];
                    out.extend(self.lower_using_decl(&single)?);
                    if let Pat::Ident(binding) = &declarator.name {
                        let flag = self.bind_local("__thaw_using_acquired", HirType::Bool);
                        using_flags.push(HirStmt::Let(flag.clone(), HirType::Bool,
                            HirExpr::Lit(HirLit::Bool(false))));
                        out.push(HirStmt::Expr(HirExpr::Assign(flag.clone(),
                            Box::new(HirExpr::Lit(HirLit::Bool(true))))));
                        using_starts.push(out.len());
                        let disposal = self.lower_using_disposal(
                            binding.id.sym.as_ref(),
                            using_decl.is_await,
                        )?;
                        using_disposals.push(Self::guard_using_disposal(flag, disposal));
                    }
                }
            } else {
                out.extend(self.lower_stmt_seq(stmt)?);
            }
            if let Stmt::Expr(expression) = stmt {
                if let Some(targets) = self.assertion_union_narrowing(&expression.expr) {
                    for target in targets {
                        self.union_narrowings
                            .insert(target.name, (target.matching, target.elements));
                    }
                }
            }
            if let Stmt::If(if_stmt) = stmt {
                let condition_narrowing = self.last_if_condition_narrowing.take();
                if if_stmt.alt.is_none() && Self::stmt_definitely_exits(&if_stmt.cons) {
                    if let Some((name, payload, present_when_true, absence_kind)) =
                        condition_narrowing
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
                                3 => {
                                    self.partial_nullable_narrowings.insert(name, payload);
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
                    if let Some((name, _, Some(continuing_type))) =
                        self.json_typeof_narrowing(&if_stmt.test)
                    {
                        self.json_narrowings.insert(name, continuing_type);
                    }
                }
            }
        }
        self.lower_using_scope(out, using_disposals, using_flags, using_starts)
    }

    fn lower_using_scope(
        &mut self,
        out: Vec<HirStmt>,
        mut using_disposals: Vec<HirStmt>,
        mut using_flags: Vec<HirStmt>,
        using_starts: Vec<usize>,
    ) -> Result<Vec<HirStmt>, String> {
        if using_disposals.is_empty() {
            return Ok(out);
        }
        let declared_order = using_disposals.clone();
        using_disposals.reverse();
        let mut name = "__thaw_using_finally".to_string();
        while self.scope.contains_key(&name) {
            name.push('_');
        }
        let catch_name = self.bind_local(&name, crate::caught_exception_carrier_type());
        let normal_disposal = self.lower_using_disposal_sequence(&using_disposals, None)?;
        let exceptional_disposal = self.lower_using_disposal_sequence(
            &using_disposals,
            Some(HirExpr::Var(catch_name.clone())),
        )?;
        self.generator_finalizers
            .insert(catch_name.clone(), normal_disposal.clone());
        // An exit only disposes the resources already acquired before it: a
        // later resource's binding does not exist yet at an earlier `return`.
        let mut body = Vec::new();
        let mut exposed = Vec::new();
        let mut rest = out;
        let mut consumed = 0;
        let mut boundaries = using_starts.clone();
        boundaries.push(consumed + rest.len());
        for (live, end) in boundaries.into_iter().enumerate() {
            let segment: Vec<HirStmt> = rest.drain(..end - consumed).collect();
            consumed = end;
            let segment = if live == 0 {
                segment
            } else {
                let live_disposals: Vec<HirStmt> =
                    declared_order[..live].iter().rev().cloned().collect();
                let disposal = self.lower_using_disposal_sequence(&live_disposals, None)?;
                self.inject_finally_before_exits(segment, &disposal, false, 1)?
            };
            for stmt in &segment {
                if let HirStmt::Let(name, ty, _) = stmt {
                    if !self.scope.contains_key(name) {
                        self.scope.insert(name.clone(), ty.clone());
                        exposed.push(name.clone());
                    }
                }
            }
            body.extend(segment);
        }
        for name in exposed {
            self.scope.remove(&name);
        }
        // Every flag is initialized before any fallible acquisition in the Try.
        using_flags.push(crate::caught_exception_carrier_prelude(catch_name.clone()));
        using_flags.push(HirStmt::Try(
            body,
            catch_name.clone(),
            exceptional_disposal,
            None,
        ));
        using_flags.extend(normal_disposal);
        Ok(using_flags)
    }

    fn guard_using_disposal(flag: Symbol, disposal: HirStmt) -> HirStmt {
        HirStmt::If(HirExpr::Var(flag.clone()), vec![
            // Consume before calling user code: a throwing disposer must not
            // be attempted again by the enclosing exceptional cleanup.
            HirStmt::Expr(HirExpr::Assign(flag,
                Box::new(HirExpr::Lit(HirLit::Bool(false))))),
            disposal,
        ], Vec::new())
    }

    /// A nesting-safe `SuppressedError` combining a new `error` with the
    /// pending error, matching `new SuppressedError(error, suppressed)`.
    fn suppressed_error_expr(
        &mut self, error: HirExpr, suppressed: HirExpr,
    ) -> Result<HirExpr, String> {
        let carrier = crate::caught_exception_carrier_type();
        let HirType::Union(members) = carrier else { unreachable!() };
        let record = self.lower_trusted_error_fields(vec![
            ("__thaw_class_identity_\u{1e}SuppressedError\u{1f}Error".into(),
                HirExpr::Lit(HirLit::Bool(true))),
            ("message".into(), HirExpr::Lit(HirLit::Str(
                "An error was suppressed during disposal".into(),
            ))),
            ("name".into(), HirExpr::Lit(HirLit::Str("SuppressedError".into()))),
            ("error".into(), error),
            ("suppressed".into(), suppressed),
        ], HirExpr::Lit(HirLit::Bool(false)), None)?;
        let record_type = self.infer_expr_type(&record)?;
        let HirType::Object(_) = &record_type else {
            return Err("trusted SuppressedError record must have a fixed object layout".into());
        };
        let owner = self.bind_local("__thaw_suppressed_error_owner", record_type.clone());
        let property = |name: &str| HirExpr::PropAccess(
            Box::new(HirExpr::Var(owner.clone())), record_type.clone(), name.into(),
        );
        let message = self.coerce_to_declared(&HirType::Json, property("message"))?;
        let name = self.coerce_to_declared(&HirType::Json, property("name"))?;
        let error = self.caught_carrier_as_json(property("error"))?;
        let suppressed = self.caught_carrier_as_json(property("suppressed"))?;
        let snapshot = HirExpr::JsonObjectLit(vec![
            ("message".into(), message),
            ("name".into(), name),
            ("error".into(), error),
            ("suppressed".into(), suppressed),
        ], HirType::Json);
        let describer = HirExpr::Lambda(
            vec![HirParam { name: owner.clone(), ty: record_type.clone() }],
            Vec::new(), HirType::Json, Box::new(snapshot),
        );
        let layout = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_register_native_object_layout".into())),
            vec![
                HirExpr::Var(owner.clone()),
                HirExpr::Lit(HirLit::Bool(false)),
            ],
        );
        let register_describer = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_register_native_exception_describer".into())),
            vec![HirExpr::Var(owner.clone()), describer],
        );
        let carrier = HirExpr::UnionInject(Box::new(HirExpr::TypedClosure(
            HirType::Object(Vec::new()), Box::new(HirExpr::Var(owner.clone())),
        )), 6, members);
        let result = HirExpr::EvalThen(
            Box::new(HirExpr::EvalThen(Box::new(layout), Box::new(register_describer))),
            Box::new(carrier),
        );
        self.wrap_call_argument_bindings(result, &[(owner, record_type, record)])
    }

    /// Dispose every resource even if an earlier disposer throws. When an
    /// exception is already pending, or a second disposer also fails, expose
    /// the combined completion as a `SuppressedError` on thaw's tagged-string
    /// exception channel.
    fn lower_using_disposal_sequence(
        &mut self,
        disposals: &[HirStmt],
        pending: Option<HirExpr>,
    ) -> Result<Vec<HirStmt>, String> {
        let has_pending = pending.is_some();
        let carrier = crate::caught_exception_carrier_type();
        let HirType::Union(members) = &carrier else { unreachable!() };
        let pending_name = self.bind_local("__thaw_using_pending", carrier.clone());
        let has_pending_name = self.bind_local("__thaw_using_has_pending", HirType::Bool);
        let mut statements = vec![
            HirStmt::Let(
                pending_name.clone(),
                carrier.clone(),
                pending.unwrap_or_else(|| HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(HirLit::Undefined)), 4, members.clone(),
                )),
            ),
            HirStmt::Let(
                has_pending_name.clone(),
                HirType::Bool,
                HirExpr::Lit(HirLit::Bool(has_pending)),
            ),
        ];
        for disposal in disposals {
            let error_name = self.bind_local("__thaw_using_dispose_error", carrier.clone());
            let combined = self.suppressed_error_expr(
                HirExpr::Var(error_name.clone()), HirExpr::Var(pending_name.clone()),
            )?;
            statements.push(crate::caught_exception_carrier_prelude(error_name.clone()));
            statements.push(HirStmt::Try(
                vec![disposal.clone()],
                error_name.clone(),
                vec![
                    HirStmt::If(
                        HirExpr::Var(has_pending_name.clone()),
                        vec![HirStmt::Expr(HirExpr::Assign(
                            pending_name.clone(),
                            Box::new(combined),
                        ))],
                        vec![HirStmt::Expr(HirExpr::Assign(
                            pending_name.clone(),
                            Box::new(HirExpr::Var(error_name.clone())),
                        ))],
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        has_pending_name.clone(),
                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                    )),
                ],
                None,
            ));
        }
        if has_pending {
            statements.extend(self.lower_throw_value_statements(
                HirExpr::Var(pending_name), carrier,
            )?);
        } else {
            let rethrow = self.lower_throw_value_statements(
                HirExpr::Var(pending_name), carrier,
            )?;
            statements.push(HirStmt::If(
                HirExpr::Var(has_pending_name),
                rethrow,
                Vec::new(),
            ));
        }
        Ok(statements)
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
        let mut values = Vec::new();
        let mut bare = false;
        collect_return_values(body, &mut values, &mut bare);
        if values.is_empty() {
            return Ok(HirType::Void);
        }
        if bare {
            return Err("function mixes value-returning and bare `return` statements".into());
        }
        // A body that only returns byte buffers returns a `Bytes` (not its erased `number[]`).
        if values.iter().all(|value| self.infer_expr_type_inner(value).is_ok_and(|ty| ty == HirType::Bytes)) {
            return Ok(HirType::Bytes);
        }
        let mut members = Vec::new();
        for value in &values {
            let ty = self.infer_expr_type(value)?;
            if ty == HirType::Dynamic {
                return Ok(HirType::Dynamic);
            }
            if !members.contains(&ty) {
                members.push(ty);
            }
        }
        match members.len() {
            1 => Ok(members.into_iter().next().unwrap()),
            _ => Ok(HirType::Union(members)),
        }
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
                    HirStmt::Finally(body, _) => {
                        collect(body, yields, returns, yielded, returned);
                    }
                    HirStmt::Try(try_body, _, catch_body, _) => {
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
        self.source_break_depths.push(self.loop_depth);
        self.source_continue_depths.push(self.loop_depth);
        let result = self.lower_body(stmt);
        self.source_continue_depths.pop();
        self.source_break_depths.pop();
        self.loop_depth -= 1;
        result
    }

    fn lower_body_with_optional_narrowing(
        &mut self,
        stmt: &Stmt,
        narrowing: Option<&(Symbol, HirType, u8)>,
    ) -> Result<Vec<HirStmt>, String> {
        let saved_narrowings = self.save_narrowings();
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
                3 => {
                    self.partial_nullable_narrowings
                        .insert(name.clone(), payload.clone());
                }
                _ => unreachable!(),
            }
        }
        let lowered = self.lower_body(stmt);
        self.restore_narrowings(saved_narrowings);
        lowered
    }

    fn lower_expr_with_optional_narrowing(
        &mut self,
        expr: &Expr,
        narrowing: Option<&(Symbol, HirType, u8)>,
    ) -> Result<HirExpr, String> {
        let saved_narrowings = self.save_narrowings();
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
                3 => {
                    self.partial_nullable_narrowings
                        .insert(name.clone(), payload.clone());
                }
                _ => unreachable!(),
            }
        }
        let lowered = self.lower_expr(expr);
        self.restore_narrowings(saved_narrowings);
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

    fn union_subject_path(&self, expr: &Expr) -> Option<Symbol> {
        let mut properties: Vec<String> = Vec::new();
        let mut current = expr;
        loop {
            match current {
                Expr::Ident(identifier) => {
                    let mut path = self.resolve_binding(identifier.sym.as_ref());
                    for property in properties.into_iter().rev() {
                        path.push('.');
                        path.push_str(&property);
                    }
                    return Some(path);
                }
                Expr::Paren(value) => current = &value.expr,
                Expr::TsNonNull(value) => current = &value.expr,
                Expr::Member(member) => {
                    properties.push(member_property_name(&member.prop)?);
                    current = &member.obj;
                }
                _ => return None,
            }
        }
    }

    fn union_subject_type(&self, expr: &Expr) -> Option<HirType> {
        let mut properties: Vec<String> = Vec::new();
        let mut current = expr;
        let mut binding = loop {
            match current {
                Expr::Ident(identifier) => {
                    break self
                        .scope
                        .get(&self.resolve_binding(identifier.sym.as_ref()))?
                        .clone();
                }
                Expr::Paren(value) => current = &value.expr,
                Expr::TsNonNull(value) => current = &value.expr,
                Expr::Member(member) => {
                    properties.push(member_property_name(&member.prop)?);
                    current = &member.obj;
                }
                _ => return None,
            }
        };
        for property in properties.into_iter().rev() {
            let HirType::Object(fields) = binding else {
                return None;
            };
            binding = Self::fixed_object_property_read_type(&fields, &property).ok()?;
        }
        Some(binding)
    }

    fn union_subject_elements(&self, expr: &Expr, path: &str) -> Option<Vec<HirType>> {
        if let Some((_, elements)) = self.union_narrowings.get(path) {
            return Some(elements.clone());
        }
        match self.union_subject_type(expr)? {
            HirType::Union(elements) => Some(elements),
            HirType::Optional(payload) => match *payload {
                HirType::Union(elements) => Some(elements),
                _ => None,
            },
            _ => None,
        }
    }

    fn narrow_union_value(
        &self,
        path: &str,
        value: HirExpr,
        value_type: &HirType,
    ) -> HirExpr {
        let Some(([index], elements)) = self
            .union_narrowings
            .get(path)
            .map(|(allowed, elements)| (allowed.as_slice(), elements))
        else {
            return value;
        };
        if value_type == &HirType::Union(elements.clone()) {
            return HirExpr::UnionValue(
                Box::new(value),
                *index,
                elements.clone(),
            );
        }
        if value_type == &HirType::Optional(Box::new(HirType::Union(elements.clone()))) {
            let unwrapped = HirExpr::OptionalValue(
                Box::new(value),
                HirType::Union(elements.clone()),
            );
            return HirExpr::UnionValue(
                Box::new(unwrapped),
                *index,
                elements.clone(),
            );
        }
        value
    }

    fn lower_narrowed_fixed_object_property_read(
        &mut self,
        object: HirExpr,
        member: &MemberExpr,
        fields: &[(Symbol, HirType)],
        property: &str,
    ) -> Result<HirExpr, String> {
        let expression = Expr::Member(member.clone());
        let Some(path) = self.union_subject_path(&expression) else {
            return self.lower_fixed_object_property_read(object, fields, property);
        };
        if !self.union_narrowings.contains_key(&path) {
            return self.lower_fixed_object_property_read(object, fields, property);
        }
        let value_type = Self::fixed_object_property_read_type(fields, property)?;
        let value = self.lower_fixed_object_property_read(object, fields, property)?;
        Ok(self.narrow_union_value(&path, value, &value_type))
    }

    fn update_union_narrowings_for_write(
        &mut self,
        path: &str,
        target_type: Option<&HirType>,
        assigned_member: Option<usize>,
    ) {
        let invalidated = self
            .union_narrowings
            .keys()
            .filter(|subject| {
                if subject.as_str() == path || subject.starts_with(&format!("{path}.")) {
                    return true;
                }
                let Some(suffix) = path.strip_prefix(&format!("{subject}.")) else {
                    return false;
                };
                let property = suffix.split('.').next().unwrap_or_default();
                self.union_discriminants
                    .get(*subject)
                    .is_some_and(|properties| properties.contains_key(property))
            })
            .cloned()
            .collect::<Vec<_>>();
        for subject in invalidated {
            self.union_narrowings.remove(&subject);
        }
        if let (Some(HirType::Union(elements)), Some(index)) = (target_type, assigned_member) {
            if elements.get(index).is_some() {
                self.union_narrowings
                    .insert(path.to_string(), (vec![index], elements.clone()));
            }
        }
    }

    /// Returns the optional binding tested by an absence check and the branch
    /// where narrowing applies. Kind 3 keeps null after excluding undefined
    /// from a Nullish payload.
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
        // A bare `if (x)` on an `Optional`/`Nullable`/`Nullish` value:
        // truthy => the payload is present (narrow the then-branch, or
        // via `!x`/the guard-clause path, the else-branch / rest of the
        // function). The other branch is *not* narrowed -- a present but
        // falsy payload (`0`, `""`, `false`) also fails the test.
        if let Expr::Ident(ident) = expr {
            let name = self.resolve_binding(ident.sym.as_ref());
            let absence_kind = match self.scope.get(&name)? {
                HirType::Optional(_) => 0,
                HirType::Nullable(_) => 1,
                HirType::Nullish(_) => 2,
                _ => return None,
            };
            let payload = match self.scope.get(&name)? {
                HirType::Optional(payload)
                | HirType::Nullable(payload)
                | HirType::Nullish(payload) => payload.as_ref().clone(),
                _ => unreachable!(),
            };
            return Some((name, payload, true, absence_kind));
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
        let typeof_ident_and_lit = match (binary.left.as_ref(), binary.right.as_ref()) {
            (Expr::Unary(unary), Expr::Lit(Lit::Str(value)))
            | (Expr::Lit(Lit::Str(value)), Expr::Unary(unary))
                if unary.op == UnaryOp::TypeOf =>
            {
                match unary.arg.as_ref() {
                    Expr::Ident(ident) => Some((ident, value)),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((ident, type_lit)) = typeof_ident_and_lit {
            let name = self.resolve_binding(ident.sym.as_ref());
            let (payload, absence_kind) = match self.scope.get(&name)? {
                HirType::Optional(payload) => (payload.as_ref().clone(), 0u8),
                HirType::Nullable(payload) => (payload.as_ref().clone(), 1),
                HirType::Nullish(payload) => (payload.as_ref().clone(), 2),
                _ => return None,
            };
            // Excluding undefined from Nullish<T> leaves T | null. Keep
            // the null tag until a later guard excludes it too.
            if type_lit.value == *"undefined" && absence_kind != 1 {
                return Some((name, payload, present_when_true,
                    if absence_kind == 2 { 3 } else { absence_kind }));
            }
            if native_typeof_name(&payload).is_some_and(|kind| type_lit.value == *kind) {
                // JavaScript reports null as "object", so that test cannot
                // extract an object payload from Nullable/Nullish.
                if type_lit.value == *"object" && absence_kind != 0 {
                    return (absence_kind == 2).then_some((name, payload, !present_when_true, 3));
                }
                return Some((name, payload, !present_when_true, absence_kind));
            }
            return None;
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
                if matches!(binary.op, BinaryOp::EqEq | BinaryOp::NotEq)
                    || (self.partial_nullable_narrowings.contains_key(&name) && nullable) =>
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
        let (subject, property) = match target {
            Expr::Ident(_) => (target, None),
            Expr::Member(member) => {
                (
                    member.obj.as_ref(),
                    Some(member_property_name(&member.prop)?),
                )
            }
            _ => return None,
        };
        let name = self.union_subject_path(subject)?;
        let elements = self.union_subject_elements(subject, &name)?;
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

    fn json_typeof_narrowing(
        &self,
        expr: &Expr,
    ) -> Option<(Symbol, Option<HirType>, Option<HirType>)> {
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
            _ => {
                let (target, compares_null) = match (binary.left.as_ref(), binary.right.as_ref()) {
                    (Expr::Ident(target), Expr::Lit(Lit::Null(_)))
                    | (Expr::Lit(Lit::Null(_)), Expr::Ident(target)) => (target, true),
                    (Expr::Ident(target), Expr::Ident(undefined))
                    | (Expr::Ident(undefined), Expr::Ident(target))
                        if undefined.sym == *"undefined"
                            && !self.scope.contains_key("undefined") =>
                    {
                        (target, false)
                    }
                    _ => return None,
                };
                let name = self.resolve_binding(target.sym.as_ref());
                if self.scope.get(&name) != Some(&HirType::Json) {
                    return None;
                }
                let (payload, allows_absence_check) = match self.source_scope.get(&name)? {
                    HirType::Optional(payload) => (
                        payload.as_ref(),
                        !compares_null || matches!(binary.op, BinaryOp::EqEq | BinaryOp::NotEq),
                    ),
                    HirType::Nullable(payload) => (
                        payload.as_ref(),
                        compares_null || matches!(binary.op, BinaryOp::EqEq | BinaryOp::NotEq),
                    ),
                    HirType::Nullish(payload) => (
                        payload.as_ref(),
                        matches!(binary.op, BinaryOp::EqEq | BinaryOp::NotEq),
                    ),
                    _ => return None,
                };
                if !allows_absence_check
                    || !matches!(payload, HirType::F64 | HirType::Str | HirType::Bool)
                {
                    return None;
                }
                let (then_type, else_type) = if equal_when_true {
                    (None, Some(payload.clone()))
                } else {
                    (Some(payload.clone()), None)
                };
                return Some((name, then_type, else_type));
            }
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
        let mut then_type = None;
        let mut else_type = None;
        if equal_when_true {
            then_type = Some(narrowed.clone());
        } else {
            else_type = Some(narrowed.clone());
        }
        let source = self
            .json_narrowings
            .get(&name)
            .or_else(|| self.source_scope.get(&name));
        if let Some(HirType::Union(elements)) = source {
            let matches = elements
                .iter()
                .filter(|element| native_typeof_name(element) == Some(type_name.as_ref()))
                .collect::<Vec<_>>();
            let complement = elements
                .iter()
                .filter(|element| native_typeof_name(element) != Some(type_name.as_ref()))
                .collect::<Vec<_>>();
            if matches.len() == 1 && complement.len() == 1 {
                let (matched, other) = (matches[0].clone(), complement[0].clone());
                if equal_when_true {
                    then_type = Some(matched.clone());
                    else_type = Some(other.clone());
                } else {
                    then_type = Some(other.clone());
                    else_type = Some(matched.clone());
                }
            } else if matches.len() == 1 && complement.len() > 1 {
                let matched = matches[0].clone();
                if equal_when_true {
                    then_type = Some(matched);
                } else {
                    else_type = Some(matched);
                }
            }
        }
        Some((name, then_type, else_type))
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
                Some(hir_string_literal_from_wtf8(value.value.as_wtf8().as_bytes()))
            }
            Expr::Lit(Lit::Bool(value)) => Some(HirLit::Bool(value.value)),
            _ => None,
        };
        let property_target = |value: &Expr| {
            let Expr::Member(member) = value else {
                return None;
            };
            let name = self.union_subject_path(&member.obj)?;
            let elements = self.union_subject_elements(&member.obj, &name)?;
            let property = member_property_name(&member.prop)?;
            Some((
                name,
                property,
                elements,
            ))
        };
        let property_comparison = property_target(&binary.left)
            .zip(literal_value(&binary.right))
            .or_else(|| property_target(&binary.right).zip(literal_value(&binary.left)));
        if let Some(((name, property, elements), literal)) = property_comparison {
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
                        elements,
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
        let elements = match self.scope.get(&name)? {
            HirType::Union(elements) => elements,
            HirType::Optional(payload) => match payload.as_ref() {
                HirType::Union(elements) => elements,
                _ => return None,
            },
            _ => return None,
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
        let property = member_property_name(&member.prop)?;
        let name = self.union_subject_path(&member.obj)?;
        let elements = self.union_subject_elements(&member.obj, &name)?;
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
            .filter(|index| {
                let element = &elements[*index];
                // `x instanceof Map` / `Set` / `Array`: the native collection (or a subclass of it).
                let builtin = |ty: &HirType| match class.sym.as_ref() {
                    "Map" => matches!(ty, HirType::Map(..)),
                    "Set" => matches!(ty, HirType::Set(_)),
                    "Array" => matches!(ty, HirType::Array(_)),
                    _ => false,
                };
                if !self.interfaces.contains_key(class.sym.as_ref())
                    && (builtin(element) || native_base_type(element).is_some_and(builtin))
                {
                    return true;
                }
                class_type_has_identity(element, class.sym.as_ref())
            })
            .collect::<Vec<_>>();
        // Same-layout ordinary objects can fail the runtime identity check, so
        // the matching members stay possible in the false branch only when
        // another allowed member shares their physical layout (identity marker
        // aside); with distinct layouts the false branch is exactly the rest.
        let layout = |index: usize| match &elements[index] {
            HirType::Object(fields) => HirType::Object(fields.iter().skip(1).cloned().collect()),
            other => other.clone(),
        };
        let layouts_collide = allowed.iter().filter(|index| !matching.contains(index)).any(|other| {
            matching.iter().any(|matched| layout(*matched) == layout(*other))
        });
        (!matching.is_empty()).then(|| {
            (
                vec![UnionNarrowingTarget {
                    name,
                    matching,
                    allowed,
                    elements: elements.clone(),
                }],
                true,
                !layouts_collide,
            )
        })
    }

    /// `Array.isArray(x)` on a union: keeps the array members (and `extends Array` instances).
    fn union_is_array_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let Expr::Call(call) = expr else { return None };
        let Callee::Expr(callee) = &call.callee else { return None };
        let Expr::Member(member) = callee.as_ref() else { return None };
        let (Expr::Ident(object), MemberProp::Ident(method)) = (member.obj.as_ref(), &member.prop) else {
            return None;
        };
        if object.sym != *"Array" || method.sym != *"isArray" || self.interfaces.contains_key("Array") {
            return None;
        }
        let [argument] = call.args.as_slice() else { return None };
        let Expr::Ident(value) = argument.expr.as_ref() else { return None };
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
                let element = &elements[*index];
                matches!(element, HirType::Array(_))
                    || matches!(native_base_type(element), Some(HirType::Array(_)))
            })
            .collect::<Vec<_>>();
        (!matching.is_empty()).then(|| {
            (
                vec![UnionNarrowingTarget { name, matching, allowed, elements: elements.clone() }],
                true,
                true,
            )
        })
    }

    /// `x instanceof Derived` where `x` is a native instance of a base class: the guarded
    /// branch sees `x` as `Derived` (same pointer; a derived layout extends its base's).
    fn class_downcast_narrowing(&self, expr: &Expr) -> Option<(String, Symbol, HirType)> {
        let Expr::Bin(binary) = expr else { return None };
        if binary.op != BinaryOp::InstanceOf {
            return None;
        }
        let (Expr::Ident(value), Expr::Ident(class)) = (binary.left.as_ref(), binary.right.as_ref())
        else {
            return None;
        };
        let current = self.resolve_binding(value.sym.as_ref());
        let base = self.scope.get(&current)?;
        let derived = self.interfaces.get(class.sym.as_ref())?;
        if class_name_from_type(derived) != Some(class.sym.as_ref()) {
            return None;
        }
        // An interface-typed object (no class identity of its own) narrows to the class too.
        let applies = match class_name_from_type(base) {
            Some(base_name) => derived != base && class_type_has_identity(derived, base_name),
            None => matches!(base, HirType::Object(fields) if !fields.is_empty()),
        };
        applies.then(|| (value.sym.to_string(), current, derived.clone()))
    }

    /// Rebinds `source` to a `derived`-typed local aliasing the same instance.
    fn begin_class_downcast(&mut self, narrowing: &(String, Symbol, HirType)) -> (Symbol, HirStmt) {
        let (source, current, derived) = narrowing;
        let mut value = HirExpr::Var(current.clone());
        if let Some(base) = self.scope.get(current).cloned().filter(|base| class_name_from_type(base).is_none()) {
            // Interface-typed: continue with the instance behind the value.
            if let Ok(unwrapped) = self.view_unwrap(value.clone(), &base) {
                value = unwrapped;
            }
        }
        let hir_name = self.bind_local(source, derived.clone());
        (hir_name.clone(), HirStmt::Let(hir_name, derived.clone(), value))
    }

    fn end_class_downcast(&mut self, source: &str) {
        if let Some(stack) = self.bindings.get_mut(source) {
            stack.pop();
        }
    }

    fn exception_instanceof_narrowing(&self, expr: &Expr) -> Option<(Symbol, HirType)> {
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
        if self.scope.get(&name) != Some(&HirType::Str) {
            return None;
        }
        let target = self.interfaces.get(class.sym.as_ref())?.clone();
        matches!(target, HirType::Object(_)).then_some((name, target))
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
            .or_else(|| self.union_is_array_narrowing(expr))
            .or_else(|| self.union_in_narrowing(expr))
            .or_else(|| self.union_truthy_narrowing(expr))
    }

    /// A bare `if (x)` on a union that includes `null` / `undefined`:
    /// the truthy branch drops those members. (A present-but-falsy
    /// member -- `0` / `""` / `false` -- also fails the test, but thaw
    /// doesn't track literal falsiness, so every other member is kept.)
    fn union_truthy_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let Expr::Ident(ident) = expr else {
            return None;
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
            .filter(|index| !matches!(elements[*index], HirType::Null | HirType::Undefined))
            .collect::<Vec<_>>();
        (matching.len() < allowed.len() && !matching.is_empty()).then(|| {
            (
                vec![UnionNarrowingTarget {
                    name,
                    matching,
                    allowed,
                    elements: elements.clone(),
                }],
                true,
                false,
            )
        })
    }

    fn lower_body_with_union_narrowing(
        &mut self,
        stmt: &Stmt,
        narrowing: Option<&[UnionNarrowingTarget]>,
        optional: Option<&(Symbol, HirType, u8)>,
        json: Option<&(Symbol, HirType)>,
        exception: Option<&(Symbol, HirType)>,
    ) -> Result<Vec<HirStmt>, String> {
        let saved = self.union_narrowings.clone();
        let saved_json = self.json_narrowings.clone();
        let saved_exceptions = self.exception_object_narrowings.clone();
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
        if let Some((name, ty)) = exception {
            self.exception_object_narrowings
                .insert(name.clone(), ty.clone());
        }
        let lowered = self.lower_body_with_optional_narrowing(stmt, optional);
        self.union_narrowings = saved;
        self.json_narrowings = saved_json;
        self.exception_object_narrowings = saved_exceptions;
        lowered
    }

}

#[cfg(test)]
mod thaw_binding_helper_controls {
    use super::*;

    #[test]
    fn thaw_binding_helper_declaration_flags_follow_resolved_symbols() {
        let signatures = HashMap::<Symbol, FnSignature>::new();
        let interfaces = HashMap::<Symbol, HirType>::new();
        let generic_interfaces = GenericInterfaces::new();
        let enum_values = EnumValues::new();
        let enum_reverse_values = EnumReverseValues::new();
        let mut lowerer = FnLowerer::new(
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            HirType::Void,
            None,
        );

        let outer_const = lowerer.bind_decl_local("value", HirType::F64, true);
        assert_eq!(lowerer.resolve_binding("value"), outer_const);
        assert!(lowerer.immutable_bindings.contains(&outer_const));

        let mutable_shadow = lowerer.bind_decl_local("value", HirType::F64, false);
        assert_ne!(mutable_shadow, outer_const);
        assert_eq!(lowerer.resolve_binding("value"), mutable_shadow);
        assert!(lowerer.immutable_bindings.contains(&outer_const));
        assert!(!lowerer.immutable_bindings.contains(&mutable_shadow));

        let const_shadow = lowerer.bind_decl_local("value", HirType::F64, true);
        assert_ne!(const_shadow, mutable_shadow);
        assert_eq!(lowerer.resolve_binding("value"), const_shadow);
        assert!(lowerer.immutable_bindings.contains(&outer_const));
        assert!(!lowerer.immutable_bindings.contains(&mutable_shadow));
        assert!(lowerer.immutable_bindings.contains(&const_shadow));

        // `using` is intentionally routed through a synthetic `const` declaration.
        let using_binding = lowerer.bind_decl_local("resource", HirType::Json, true);
        assert!(lowerer.immutable_bindings.contains(&using_binding));

        let temporary = lowerer.bind_local("temporary", HirType::Bool);
        assert_eq!(lowerer.resolve_binding("temporary"), temporary);
        assert!(!lowerer.immutable_bindings.contains(&temporary));
    }

    #[test]
    fn thaw_binding_helper_iteration_cell_inherits_only_current_outer_immutability() {
        let signatures = HashMap::<Symbol, FnSignature>::new();
        let interfaces = HashMap::<Symbol, HirType>::new();
        let generic_interfaces = GenericInterfaces::new();
        let enum_values = EnumValues::new();
        let enum_reverse_values = EnumReverseValues::new();
        let mut lowerer = FnLowerer::new(
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            HirType::Void,
            None,
        );

        let const_outer = lowerer.bind_decl_local("const_index", HirType::F64, true);
        let const_inner = lowerer.bind_for_iteration_local("const_index", HirType::F64);
        assert_ne!(const_inner, const_outer);
        assert!(const_inner.starts_with("@@thaw_for_iteration_"));
        assert_eq!(lowerer.resolve_binding("const_index"), const_inner);
        assert!(lowerer.immutable_bindings.contains(&const_outer));
        assert!(lowerer.immutable_bindings.contains(&const_inner));

        let shadowed_const = lowerer.bind_decl_local("shadowed_index", HirType::F64, true);
        let current_mutable_shadow =
            lowerer.bind_decl_local("shadowed_index", HirType::F64, false);
        assert_ne!(shadowed_const, current_mutable_shadow);
        let mutable_shadow_inner =
            lowerer.bind_for_iteration_local("shadowed_index", HirType::F64);
        assert_eq!(lowerer.resolve_binding("shadowed_index"), mutable_shadow_inner);
        assert!(lowerer.immutable_bindings.contains(&shadowed_const));
        assert!(!lowerer.immutable_bindings.contains(&current_mutable_shadow));
        assert!(!lowerer.immutable_bindings.contains(&mutable_shadow_inner));

        let mutable_source =
            lowerer.bind_decl_local("renamed_const_index", HirType::F64, false);
        let renamed_const =
            lowerer.bind_decl_local("renamed_const_index", HirType::F64, true);
        assert_ne!(renamed_const, "renamed_const_index");
        assert_ne!(renamed_const, mutable_source);
        let renamed_const_inner =
            lowerer.bind_for_iteration_local("renamed_const_index", HirType::F64);
        assert_eq!(lowerer.resolve_binding("renamed_const_index"), renamed_const_inner);
        assert!(lowerer.immutable_bindings.contains(&renamed_const));
        assert!(lowerer.immutable_bindings.contains(&renamed_const_inner));
        assert!(!lowerer.immutable_bindings.contains(&mutable_source));

        let let_outer = lowerer.bind_decl_local("let_index", HirType::F64, false);
        let let_inner = lowerer.bind_for_iteration_local("let_index", HirType::F64);
        assert_ne!(let_inner, let_outer);
        assert_eq!(lowerer.resolve_binding("let_index"), let_inner);
        assert!(!lowerer.immutable_bindings.contains(&let_outer));
        assert!(!lowerer.immutable_bindings.contains(&let_inner));
    }

    #[test]
    fn thaw_source_target_stacks_restore_after_nested_lowering_errors() {
        fn first_statement(source: &str) -> Stmt {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let swc_ecma_ast::ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = &module.body[0]
            else { panic!("expected a function declaration"); };
            function.function.body.as_ref().unwrap().stmts[0].clone()
        }
        let signatures = HashMap::<Symbol, FnSignature>::new();
        let interfaces = HashMap::<Symbol, HirType>::new();
        let generic_interfaces = GenericInterfaces::new();
        let enum_values = EnumValues::new();
        let enum_reverse_values = EnumReverseValues::new();
        let mut lowerer = FnLowerer::new(
            &signatures, &interfaces, &generic_interfaces, &enum_values,
            &enum_reverse_values, HirType::Void, None,
        );
        lowerer.loop_depth = 5;
        lowerer.source_break_depths.push(5);
        lowerer.source_continue_depths.push(5);
        lowerer.labels.push(("seed".into(), 5, true));
        let bindings = lowerer.bindings.clone();
        let scope = lowerer.scope.clone();
        let mut loop_error = first_statement(
            "function main() { known: while (true) { continue known; } }",
        );
        let Stmt::Labeled(label) = &mut loop_error else { panic!("expected label"); };
        let Stmt::While(loop_stmt) = label.body.as_mut() else { panic!("expected loop"); };
        let Stmt::Block(body) = loop_stmt.body.as_mut() else { panic!("expected block"); };
        let Stmt::Continue(continue_stmt) = &mut body.stmts[0] else { panic!("expected continue"); };
        continue_stmt.label.as_mut().unwrap().sym = "missing".into();

        let mut switch_error = first_statement(
            "function main() { known: while (true) { switch (1) { default: break known; } } }",
        );
        let Stmt::Labeled(label) = &mut switch_error else { panic!("expected label"); };
        let Stmt::While(loop_stmt) = label.body.as_mut() else { panic!("expected loop"); };
        let Stmt::Block(body) = loop_stmt.body.as_mut() else { panic!("expected block"); };
        let Stmt::Switch(switch_stmt) = &mut body.stmts[0] else { panic!("expected switch"); };
        let Stmt::Break(break_stmt) = &mut switch_stmt.cases[0].cons[0] else { panic!("expected break"); };
        break_stmt.label.as_mut().unwrap().sym = "missing".into();

        let mut label_error = first_statement(
            "function main() { while (true) { label: while (true) { continue label; } } }",
        );
        let Stmt::While(outer_loop) = &mut label_error else { panic!("expected outer loop"); };
        let Stmt::Block(outer_body) = outer_loop.body.as_mut() else { panic!("expected outer body"); };
        let Stmt::Labeled(label) = &mut outer_body.stmts[0] else { panic!("expected label"); };
        let Stmt::While(label_loop) = label.body.as_ref() else { panic!("expected labeled loop"); };
        label.body = label_loop.body.clone();

        for (stmt, expected) in [
            (&loop_error, "unknown continue label `missing`"),
            (&switch_error, "unknown break label `missing`"),
            (&label_error, "continue label `label` does not name a loop"),
        ] {
            assert!(lowerer.lower_stmt_seq(stmt).unwrap_err().contains(expected));
            assert_eq!(lowerer.loop_depth, 5);
            assert_eq!(lowerer.source_break_depths, [5]);
            assert_eq!(lowerer.source_continue_depths, [5]);
            assert_eq!(lowerer.labels, [("seed".into(), 5, true)]);
            assert_eq!(lowerer.bindings, bindings);
            assert_eq!(lowerer.scope, scope);
        }
    }

}
