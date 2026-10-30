// `var` hoisting for the names that need it, and the straight-line TDZ names of a
// statement list. Both are syntactic and leave every other declaration untouched.

/// Names of the `var`s declared in `stmts` (not inside closures) that are used before their
/// declaration, or outside the block that declares them. Those are hoisted to the start of the
/// list as `var name;` (an `undefined` `Json`) and their declarations become assignments.
fn pre_read_var_names<'a>(stmts: impl IntoIterator<Item = &'a Stmt>) -> Vec<String> {
    #[derive(Default)]
    struct Scan {
        frames: Vec<HashSet<String>>,
        closure_depth: usize,
        candidates: HashSet<String>,
        blocked: HashSet<String>,
        pre_reads: HashSet<String>,
    }
    impl Scan {
        fn read(&mut self, name: &str) {
            if !self.frames.iter().any(|frame| frame.contains(name)) {
                self.pre_reads.insert(name.to_string());
            }
        }
        fn closure<N: VisitWith<BoundNames> + VisitWith<Scan>>(&mut self, node: &N) {
            // Everything a closure binds (parameters, its own declarations) shadows from entry.
            let mut bound = BoundNames::default();
            node.visit_with(&mut bound);
            self.frames.push(bound.0);
            self.closure_depth += 1;
            node.visit_children_with(self);
            self.closure_depth -= 1;
            self.frames.pop();
        }
        fn declare(&mut self, declaration: &VarDecl, listed: bool) {
            for declarator in &declaration.decls {
                if let Some(init) = &declarator.init {
                    init.visit_with(self);
                }
                struct Names(Vec<String>);
                impl Visit for Names {
                    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
                        self.0.push(binding.id.sym.to_string());
                    }
                    fn visit_expr(&mut self, _: &Expr) {}
                }
                let mut names = Names(Vec::new());
                declarator.name.visit_with(&mut names);
                let simple = listed && matches!(declarator.name, Pat::Ident(_));
                for name in names.0 {
                    if self.closure_depth == 0 && !simple {
                        self.blocked.insert(name.clone());
                    } else if self.closure_depth == 0 {
                        self.candidates.insert(name.clone());
                    }
                    if let Some(frame) = self.frames.last_mut() {
                        frame.insert(name);
                    }
                }
            }
        }
    }
    impl Visit for Scan {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Decl(Decl::Var(declaration)) if declaration.kind == VarDeclKind::Var => {
                    self.declare(declaration, true);
                }
                _ => stmt.visit_children_with(self),
            }
        }
        fn visit_var_decl(&mut self, declaration: &VarDecl) {
            if declaration.kind == VarDeclKind::Var {
                self.declare(declaration, false);
            } else {
                declaration.visit_children_with(self);
                // A `let`/`const` of the same name shadows a later `var` read.
                let mut bound = BoundNames::default();
                declaration.visit_with(&mut bound);
                if let Some(frame) = self.frames.last_mut() {
                    frame.extend(bound.0);
                }
            }
        }
        fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
            self.frames.push(HashSet::new());
            block.visit_children_with(self);
            self.frames.pop();
        }
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Ident(ident) => self.read(ident.sym.as_ref()),
                _ => expr.visit_children_with(self),
            }
        }
        fn visit_assign_expr(&mut self, assign: &swc_ecma_ast::AssignExpr) {
            if let Some(ident) = assign.left.as_ident() {
                self.read(ident.sym.as_ref());
            }
            assign.visit_children_with(self);
        }
        fn visit_prop(&mut self, prop: &swc_ecma_ast::Prop) {
            match prop {
                swc_ecma_ast::Prop::Shorthand(ident) => self.read(ident.sym.as_ref()),
                _ => prop.visit_children_with(self),
            }
        }
        fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
            self.closure(function);
        }
        fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
            self.closure(arrow);
        }
        fn visit_constructor(&mut self, constructor: &swc_ecma_ast::Constructor) {
            self.closure(constructor);
        }
        fn visit_getter_prop(&mut self, getter: &swc_ecma_ast::GetterProp) {
            self.closure(getter);
        }
        fn visit_setter_prop(&mut self, setter: &swc_ecma_ast::SetterProp) {
            self.closure(setter);
        }
        fn visit_static_block(&mut self, block: &swc_ecma_ast::StaticBlock) {
            self.closure(block);
        }
        // Property keys of types are not reads.
        fn visit_ts_type_element(&mut self, _: &swc_ecma_ast::TsTypeElement) {}
    }
    let mut scan = Scan {
        frames: vec![HashSet::new()],
        ..Scan::default()
    };
    for stmt in stmts {
        stmt.visit_with(&mut scan);
    }
    let mut names: Vec<String> = scan
        .candidates
        .into_iter()
        .filter(|name| scan.pre_reads.contains(name) && !scan.blocked.contains(name))
        .collect();
    names.sort();
    names
}

/// Rewrites `stmts` so each name in `names` is declared once, up front, as `var name;` and every
/// `var name [= init]` becomes `name = init` (or nothing).
fn hoist_var_names(stmts: &[Stmt], names: &[String]) -> Vec<Stmt> {
    struct Rewrite<'a>(&'a [String]);
    impl VisitMut for Rewrite<'_> {
        fn visit_mut_function(&mut self, _: &mut swc_ecma_ast::Function) {}
        fn visit_mut_arrow_expr(&mut self, _: &mut swc_ecma_ast::ArrowExpr) {}
        fn visit_mut_class(&mut self, _: &mut swc_ecma_ast::Class) {}
        fn visit_mut_stmts(&mut self, stmts: &mut Vec<Stmt>) {
            stmts.visit_mut_children_with(self);
            let sp = swc_common::DUMMY_SP;
            let mut rebuilt = Vec::with_capacity(stmts.len());
            for stmt in std::mem::take(stmts) {
                let Stmt::Decl(Decl::Var(declaration)) = &stmt else {
                    rebuilt.push(stmt);
                    continue;
                };
                let hoisted = |declarator: &swc_ecma_ast::VarDeclarator| {
                    matches!(&declarator.name, Pat::Ident(b) if declaration.kind == VarDeclKind::Var
                        && self.0.iter().any(|n| n.as_str() == &*b.id.sym))
                };
                if !declaration.decls.iter().any(hoisted) {
                    rebuilt.push(stmt.clone());
                    continue;
                }
                for declarator in &declaration.decls {
                    if !hoisted(declarator) {
                        let mut single = (**declaration).clone();
                        single.decls = vec![declarator.clone()];
                        rebuilt.push(Stmt::Decl(Decl::Var(Box::new(single))));
                        continue;
                    }
                    let (Pat::Ident(binding), Some(init)) = (&declarator.name, &declarator.init)
                    else {
                        continue;
                    };
                    rebuilt.push(Stmt::Expr(swc_ecma_ast::ExprStmt {
                        span: sp,
                        expr: Box::new(Expr::Assign(swc_ecma_ast::AssignExpr {
                            span: sp,
                            op: swc_ecma_ast::AssignOp::Assign,
                            left: swc_ecma_ast::AssignTarget::Simple(
                                swc_ecma_ast::SimpleAssignTarget::Ident(
                                    swc_ecma_ast::BindingIdent {
                                        id: swc_ecma_ast::Ident::new_no_ctxt(
                                            binding.id.sym.clone(),
                                            sp,
                                        ),
                                        type_ann: None,
                                    },
                                ),
                            ),
                            right: init.clone(),
                        })),
                    }));
                }
            }
            *stmts = rebuilt;
        }
    }
    let sp = swc_common::DUMMY_SP;
    let mut result: Vec<Stmt> = names
        .iter()
        .map(|name| {
            Stmt::Decl(Decl::Var(Box::new(VarDecl {
                span: sp,
                ctxt: swc_common::SyntaxContext::empty(),
                kind: VarDeclKind::Var,
                declare: false,
                decls: vec![swc_ecma_ast::VarDeclarator {
                    span: sp,
                    name: Pat::Ident(swc_ecma_ast::BindingIdent {
                        id: swc_ecma_ast::Ident::new_no_ctxt(name.as_str().into(), sp),
                        type_ann: None,
                    }),
                    init: None,
                    definite: false,
                }],
            })))
        })
        .collect();
    let mut body = stmts.to_vec();
    Rewrite(names).visit_mut_stmts(&mut body);
    result.extend(body);
    result
}

/// Module level: `var`s used before their declaration are hoisted to the start of the module.
fn hoist_top_level_vars(module: &Module) -> Module {
    if module
        .body
        .iter()
        .any(|item| matches!(item, ModuleItem::ModuleDecl(_)))
    {
        return module.clone();
    }
    let stmts = module.body.iter().filter_map(|item| match item {
        ModuleItem::Stmt(stmt) => Some(stmt),
        ModuleItem::ModuleDecl(_) => None,
    });
    // Function and class bodies already see every global.
    let names = pre_read_var_names(
        stmts
            .clone()
            .filter(|stmt| !matches!(stmt, Stmt::Decl(Decl::Fn(_) | Decl::Class(_)))),
    );
    if names.is_empty() {
        return module.clone();
    }
    let stmts: Vec<Stmt> = stmts.cloned().collect();
    let mut hoisted = module.clone();
    hoisted.body = hoist_var_names(&stmts, &names)
        .into_iter()
        .map(ModuleItem::Stmt)
        .collect();
    hoisted
}

/// Module level counterpart of `closure_tdz_flags`.
fn flag_top_level_closure_tdz(module: &Module) -> Module {
    if module
        .body
        .iter()
        .any(|item| matches!(item, ModuleItem::ModuleDecl(_)))
    {
        return module.clone();
    }
    let stmts: Vec<Stmt> = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(stmt) => Some(stmt.clone()),
            ModuleItem::ModuleDecl(_) => None,
        })
        .collect();
    let Some(flagged) = closure_tdz_flags(&stmts, true) else {
        return module.clone();
    };
    let mut result = module.clone();
    result.body = flagged.into_iter().map(ModuleItem::Stmt).collect();
    result
}

/// Names bound anywhere inside a node (parameters, declarations, nested functions/classes).
#[derive(Default)]
struct BoundNames(HashSet<String>);
impl Visit for BoundNames {
    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        self.0.insert(binding.id.sym.to_string());
    }
    fn visit_fn_decl(&mut self, declaration: &swc_ecma_ast::FnDecl) {
        self.0.insert(declaration.ident.sym.to_string());
        declaration.visit_children_with(self);
    }
    fn visit_class_decl(&mut self, declaration: &swc_ecma_ast::ClassDecl) {
        self.0.insert(declaration.ident.sym.to_string());
        declaration.visit_children_with(self);
    }
}

/// Identifiers read inside closure bodies (and not shadowed there).
#[derive(Default)]
struct ClosureReads {
    shadows: Vec<HashSet<String>>,
    out: HashSet<String>,
}
impl ClosureReads {
    fn closure<N: VisitWith<BoundNames> + VisitWith<ClosureReads>>(&mut self, node: &N) {
        let mut bound = BoundNames::default();
        node.visit_with(&mut bound);
        self.shadows.push(bound.0);
        node.visit_children_with(self);
        self.shadows.pop();
    }
}
impl Visit for ClosureReads {
    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(ident)
                if !self.shadows.is_empty()
                    && !self.shadows.iter().any(|bound| bound.contains(&*ident.sym)) =>
            {
                self.out.insert(ident.sym.to_string());
            }
            _ => expr.visit_children_with(self),
        }
    }
    fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
        self.closure(function);
    }
    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        self.closure(arrow);
    }
    fn visit_constructor(&mut self, constructor: &swc_ecma_ast::Constructor) {
        self.closure(constructor);
    }
    fn visit_getter_prop(&mut self, getter: &swc_ecma_ast::GetterProp) {
        self.closure(getter);
    }
    fn visit_setter_prop(&mut self, setter: &swc_ecma_ast::SetterProp) {
        self.closure(setter);
    }
    fn visit_ts_type_element(&mut self, _: &swc_ecma_ast::TsTypeElement) {}
}

/// Rewrites closure reads of `name` to `flag ? name : <ReferenceError>`.
struct GuardReads<'a> {
    name: &'a str,
    flag: &'a str,
    depth: usize,
}
impl GuardReads<'_> {
    fn closure<N: VisitWith<BoundNames> + VisitMutWith<Self>>(&mut self, node: &mut N) {
        let mut bound = BoundNames::default();
        (&*node).visit_with(&mut bound);
        if bound.0.contains(self.name) {
            return;
        }
        self.depth += 1;
        node.visit_mut_children_with(self);
        self.depth -= 1;
    }
}
impl VisitMut for GuardReads<'_> {
    fn visit_mut_expr(&mut self, expr: &mut Expr) {
        let sp = swc_common::DUMMY_SP;
        let Expr::Ident(ident) = expr else {
            expr.visit_mut_children_with(self);
            return;
        };
        if self.depth == 0 || *ident.sym != *self.name {
            return;
        }
        let read = ident.clone();
        let message = format!(
            "Cannot access '{}' before initialization",
            display_class_name(self.name)
        );
        *expr = Expr::Cond(swc_ecma_ast::CondExpr {
            span: sp,
            test: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                self.flag.into(),
                sp,
            ))),
            cons: Box::new(Expr::Ident(read.clone())),
            alt: Box::new(Expr::Call(swc_ecma_ast::CallExpr {
                span: sp,
                ctxt: Default::default(),
                callee: Callee::Expr(Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    "@@thaw_tdz_throw".into(),
                    sp,
                )))),
                args: vec![
                    swc_ecma_ast::ExprOrSpread {
                        spread: None,
                        expr: Box::new(Expr::Ident(read)),
                    },
                    swc_ecma_ast::ExprOrSpread {
                        spread: None,
                        expr: Box::new(Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                            span: sp,
                            value: message.into(),
                            raw: None,
                        }))),
                    },
                ],
                type_args: None,
            })),
        });
    }
    fn visit_mut_function(&mut self, function: &mut swc_ecma_ast::Function) {
        self.closure(function);
    }
    fn visit_mut_arrow_expr(&mut self, arrow: &mut swc_ecma_ast::ArrowExpr) {
        self.closure(arrow);
    }
    fn visit_mut_constructor(&mut self, constructor: &mut swc_ecma_ast::Constructor) {
        self.closure(constructor);
    }
    fn visit_mut_getter_prop(&mut self, getter: &mut swc_ecma_ast::GetterProp) {
        self.closure(getter);
    }
    fn visit_mut_setter_prop(&mut self, setter: &mut swc_ecma_ast::SetterProp) {
        self.closure(setter);
    }
    fn visit_mut_ts_type_element(&mut self, _: &mut swc_ecma_ast::TsTypeElement) {}
}

/// A `let`/`const` read inside a closure that appears textually before the declaration may run
/// before the declaration does. Such a name gets a `__thaw_tdz_<name>` flag set right after its
/// declaration, and those closure reads become `flag ? name : <ReferenceError>`. A closure read
/// needs the name in scope when the closure is lowered, so (when the initializer is a literal)
/// the declaration is also moved up to just before the first such closure and assigned in place.
fn closure_tdz_flags(stmts: &[Stmt], module_level: bool) -> Option<Vec<Stmt>> {
    fn single(stmt: &Stmt) -> Option<(String, &Expr)> {
        match stmt {
            Stmt::Decl(Decl::Var(declaration)) if declaration.kind != VarDeclKind::Var => {
                match declaration.decls.as_slice() {
                    [declarator] => match (&declarator.name, &declarator.init) {
                        (Pat::Ident(binding), Some(init)) => {
                            Some((binding.id.sym.to_string(), init.as_ref()))
                        }
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        }
    }
    if !stmts.iter().any(|stmt| single(stmt).is_some()) {
        return None;
    }
    let reads: Vec<HashSet<String>> = stmts
        .iter()
        .map(|stmt| {
            let mut reads = ClosureReads::default();
            if let Stmt::Decl(Decl::Fn(declaration)) = stmt {
                reads.closure(&*declaration.function);
            } else {
                stmt.visit_with(&mut reads);
            }
            reads.out
        })
        .collect();
    if reads.iter().all(HashSet::is_empty) {
        return None;
    }
    // A statement that declares the name itself anywhere inside (a `for (let i ..)`, a parameter)
    // reads its own binding, not the later declaration.
    let bound: Vec<HashSet<String>> = stmts
        .iter()
        .map(|stmt| {
            let mut bound = BoundNames::default();
            stmt.visit_with(&mut bound);
            bound.0
        })
        .collect();
    struct Flag {
        index: usize,
        name: String,
        flag: String,
        hoist_to: Option<usize>,
    }
    let mut flags: Vec<Flag> = Vec::new();
    for (index, stmt) in stmts.iter().enumerate() {
        let Some((name, init)) = single(stmt) else {
            continue;
        };
        let Some(first) = (0..index)
            .find(|&earlier| reads[earlier].contains(&name) && !bound[earlier].contains(&name))
        else {
            continue;
        };
        let flag_name = format!("__thaw_tdz_{name}");
        if stmts
            .iter()
            .any(|other| single(other).is_some_and(|(declared, _)| declared == flag_name))
        {
            continue;
        }
        let literal = matches!(init, Expr::Lit(Lit::Num(_) | Lit::Str(_) | Lit::Bool(_)));
        let needs_scope = !module_level || !matches!(stmts[first], Stmt::Decl(Decl::Fn(_)));
        flags.push(Flag {
            index,
            flag: flag_name,
            hoist_to: (needs_scope && literal).then_some(first),
            name,
        });
    }
    if flags.is_empty() {
        return None;
    }
    let sp = swc_common::DUMMY_SP;
    let ident = |name: &str| swc_ecma_ast::Ident::new_no_ctxt(name.into(), sp);
    let variable = |kind: VarDeclKind, name: &str, init: Expr| {
        Stmt::Decl(Decl::Var(Box::new(VarDecl {
            span: sp,
            ctxt: swc_common::SyntaxContext::empty(),
            kind,
            declare: false,
            decls: vec![swc_ecma_ast::VarDeclarator {
                span: sp,
                name: Pat::Ident(swc_ecma_ast::BindingIdent {
                    id: ident(name),
                    type_ann: None,
                }),
                init: Some(Box::new(init)),
                definite: false,
            }],
        })))
    };
    let assign = |name: &str, value: Expr| {
        Stmt::Expr(swc_ecma_ast::ExprStmt {
            span: sp,
            expr: Box::new(Expr::Assign(swc_ecma_ast::AssignExpr {
                span: sp,
                op: swc_ecma_ast::AssignOp::Assign,
                left: swc_ecma_ast::AssignTarget::Simple(swc_ecma_ast::SimpleAssignTarget::Ident(
                    swc_ecma_ast::BindingIdent {
                        id: ident(name),
                        type_ann: None,
                    },
                )),
                right: Box::new(value),
            })),
        })
    };
    let mut result = Vec::with_capacity(stmts.len() + 2 * flags.len());
    for flag in &flags {
        result.push(variable(
            VarDeclKind::Let,
            &flag.flag,
            Expr::Lit(Lit::Bool(swc_ecma_ast::Bool {
                span: sp,
                value: false,
            })),
        ));
    }
    for (index, stmt) in stmts.iter().enumerate() {
        for flag in flags.iter().filter(|flag| flag.hoist_to == Some(index)) {
            let Stmt::Decl(Decl::Var(declaration)) = &stmts[flag.index] else {
                unreachable!()
            };
            let mut stub = (**declaration).clone();
            stub.kind = VarDeclKind::Let;
            result.push(Stmt::Decl(Decl::Var(Box::new(stub))));
        }
        let mut stmt = stmt.clone();
        for flag in flags
            .iter()
            .filter(|flag| flag.index > index && !bound[index].contains(&flag.name))
        {
            let mut guard = GuardReads {
                name: &flag.name,
                flag: &flag.flag,
                depth: 0,
            };
            stmt.visit_mut_with(&mut guard);
        }
        match flags.iter().find(|flag| flag.index == index) {
            Some(flag) => {
                match flag.hoist_to {
                    Some(_) => {
                        let (_, init) = single(&stmts[index]).unwrap();
                        result.push(assign(&flag.name, init.clone()));
                    }
                    None => result.push(stmt),
                }
                result.push(assign(
                    &flag.flag,
                    Expr::Lit(Lit::Bool(swc_ecma_ast::Bool {
                        span: sp,
                        value: true,
                    })),
                ));
            }
            None => result.push(stmt),
        }
    }
    Some(result)
}

/// Straight-line TDZ: in a statement list, a read of a `let`/`const` (or `new C(..)` of a class)
/// before its declaration, outside any closure, throws a `ReferenceError`. Closures are covered
/// by `closure_tdz_flags`.
fn class_tdz_throws(stmts: &[Stmt], module_level: bool) -> Option<Vec<Stmt>> {
    struct Throw<'a> {
        name: &'a str,
        /// `Some(placeholder)` for a `let`/`const` read, `None` for `new C(..)` of a class.
        placeholder: Option<Expr>,
        changed: bool,
    }
    impl Throw<'_> {
        fn throw(&mut self, fallback: Expr) -> Expr {
            self.changed = true;
            let sp = swc_common::DUMMY_SP;
            let message = format!(
                "Cannot access '{}' before initialization",
                display_class_name(self.name)
            );
            let argument = |expr: Expr| swc_ecma_ast::ExprOrSpread {
                spread: None,
                expr: Box::new(expr),
            };
            Expr::Call(swc_ecma_ast::CallExpr {
                span: sp,
                ctxt: Default::default(),
                callee: Callee::Expr(Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    "@@thaw_tdz_throw".into(),
                    sp,
                )))),
                args: vec![
                    argument(fallback),
                    argument(Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                        span: sp,
                        value: message.into(),
                        raw: None,
                    }))),
                ],
                type_args: None,
            })
        }
    }
    impl VisitMut for Throw<'_> {
        fn visit_mut_function(&mut self, _: &mut swc_ecma_ast::Function) {}
        fn visit_mut_arrow_expr(&mut self, _: &mut swc_ecma_ast::ArrowExpr) {}
        fn visit_mut_class(&mut self, _: &mut swc_ecma_ast::Class) {}
        fn visit_mut_ts_type_element(&mut self, _: &mut swc_ecma_ast::TsTypeElement) {}
        fn visit_mut_expr(&mut self, expr: &mut Expr) {
            if let Some(placeholder) = self.placeholder.clone() {
                let hit = matches!(&*expr, Expr::Ident(ident) if *ident.sym == *self.name);
                if hit {
                    *expr = self.throw(placeholder);
                } else if !matches!(&*expr, Expr::Ident(_)) {
                    expr.visit_mut_children_with(self);
                }
                return;
            }
            expr.visit_mut_children_with(self);
            let Expr::New(construction) = expr else {
                return;
            };
            if self.placeholder.is_none()
                && matches!(construction.callee.as_ref(), Expr::Ident(class) if *class.sym == *self.name)
            {
                *expr = self.throw(expr.clone());
            }
        }
    }
    let sp = swc_common::DUMMY_SP;
    let mut declared: Vec<(usize, String, Option<Expr>)> = Vec::new();
    for (index, stmt) in stmts.iter().enumerate() {
        match stmt {
            Stmt::Decl(Decl::Class(declaration)) => {
                declared.push((index, declaration.ident.sym.to_string(), None));
            }
            Stmt::Decl(Decl::Var(declaration)) if declaration.kind != VarDeclKind::Var => {
                for declarator in &declaration.decls {
                    let Pat::Ident(binding) = &declarator.name else {
                        continue;
                    };
                    // Only a placeholder for the throwing read's static type.
                    let placeholder = match declarator.init.as_deref() {
                        Some(Expr::Lit(Lit::Num(_))) => Expr::Lit(Lit::Num(swc_ecma_ast::Number {
                            span: sp,
                            value: 0.0,
                            raw: None,
                        })),
                        Some(Expr::Lit(Lit::Str(_))) => Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                            span: sp,
                            value: "".into(),
                            raw: None,
                        })),
                        Some(Expr::Lit(Lit::Bool(_))) => Expr::Lit(Lit::Bool(swc_ecma_ast::Bool {
                            span: sp,
                            value: false,
                        })),
                        _ => Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt("undefined".into(), sp)),
                    };
                    declared.push((index, binding.id.sym.to_string(), Some(placeholder)));
                }
            }
            _ => {}
        }
    }
    if declared.is_empty() {
        return None;
    }
    let mut result = stmts.to_vec();
    let mut changed = false;
    for (index, name, placeholder) in declared {
        for stmt in &mut result[..index] {
            // A forward reference between top-level bindings stays a compile error.
            if module_level && matches!(stmt, Stmt::Decl(Decl::Var(_))) {
                continue;
            }
            let mut bound = BoundNames::default();
            stmt.visit_with(&mut bound);
            if bound.0.contains(&name) {
                continue;
            }
            let mut rewrite = Throw {
                name: &name,
                placeholder: placeholder.clone(),
                changed: false,
            };
            stmt.visit_mut_with(&mut rewrite);
            changed |= rewrite.changed;
        }
    }
    changed.then_some(result)
}

/// Module level counterpart of `class_tdz_throws`.
fn throw_top_level_class_tdz(module: &Module) -> Module {
    if module
        .body
        .iter()
        .any(|item| matches!(item, ModuleItem::ModuleDecl(_)))
    {
        return module.clone();
    }
    let stmts: Vec<Stmt> = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(stmt) => Some(stmt.clone()),
            ModuleItem::ModuleDecl(_) => None,
        })
        .collect();
    let Some(rewritten) = class_tdz_throws(&stmts, true) else {
        return module.clone();
    };
    let mut result = module.clone();
    result.body = rewritten.into_iter().map(ModuleItem::Stmt).collect();
    result
}
