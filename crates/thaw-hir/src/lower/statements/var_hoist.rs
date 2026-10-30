// `var` hoisting for the names that need it, and the straight-line TDZ names of a
// statement list. Both are syntactic and leave every other declaration untouched.

/// Names of the `var`s declared in `stmts` (not inside closures) that are used before their
/// declaration, or outside the block that declares them. Those are hoisted to the start of the
/// list as `var name;` (an `undefined` `Json`) and their declarations become assignments.
fn pre_read_var_names(stmts: &[Stmt]) -> Vec<String> {
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
        fn closure(&mut self, visit: impl FnOnce(&mut Self)) {
            self.frames.push(HashSet::new());
            self.closure_depth += 1;
            visit(self);
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
            self.closure(|scan| function.visit_children_with(scan));
        }
        fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
            self.closure(|scan| arrow.visit_children_with(scan));
        }
        fn visit_constructor(&mut self, constructor: &swc_ecma_ast::Constructor) {
            self.closure(|scan| constructor.visit_children_with(scan));
        }
        fn visit_getter_prop(&mut self, getter: &swc_ecma_ast::GetterProp) {
            self.closure(|scan| getter.visit_children_with(scan));
        }
        fn visit_setter_prop(&mut self, setter: &swc_ecma_ast::SetterProp) {
            self.closure(|scan| setter.visit_children_with(scan));
        }
        fn visit_static_block(&mut self, block: &swc_ecma_ast::StaticBlock) {
            self.closure(|scan| block.visit_children_with(scan));
        }
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
