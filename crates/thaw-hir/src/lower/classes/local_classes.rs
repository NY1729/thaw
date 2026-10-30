/// Names bound directly in one function scope (params, declarations, nested
/// function/class names); does not descend into nested functions.
#[derive(Default)]
struct ScopeBindings(HashSet<String>);

impl Visit for ScopeBindings {
    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        self.0.insert(binding.id.sym.to_string());
    }
    fn visit_fn_decl(&mut self, declaration: &FnDecl) {
        self.0.insert(declaration.ident.sym.to_string());
    }
    fn visit_class_decl(&mut self, declaration: &ClassDecl) {
        self.0.insert(declaration.ident.sym.to_string());
    }
    fn visit_function(&mut self, _: &swc_ecma_ast::Function) {}
    fn visit_arrow_expr(&mut self, _: &swc_ecma_ast::ArrowExpr) {}
    fn visit_class(&mut self, _: &swc_ecma_ast::Class) {}
}

/// Every name bound anywhere inside a node, and every identifier it mentions.
#[derive(Default)]
struct BoundAndMentioned {
    bound: HashSet<String>,
    mentioned: HashSet<String>,
}

impl Visit for BoundAndMentioned {
    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        self.bound.insert(binding.id.sym.to_string());
        binding.visit_children_with(self);
    }
    fn visit_ident(&mut self, ident: &swc_ecma_ast::Ident) {
        self.mentioned.insert(ident.sym.to_string());
    }
}

struct LocalClassLifter {
    top_names: HashSet<String>,
    class_counts: HashMap<String, usize>,
    scopes: Vec<HashSet<String>>,
    lifted: Vec<ModuleItem>,
}

impl LocalClassLifter {
    fn liftable(&self, declaration: &ClassDecl) -> bool {
        let name = declaration.ident.sym.to_string();
        if self.top_names.contains(&name) || self.class_counts.get(&name) != Some(&1) {
            return false;
        }
        let mut usage = BoundAndMentioned::default();
        declaration.class.visit_with(&mut usage);
        usage.mentioned.iter().all(|mention| {
            *mention == name
                || usage.bound.contains(mention)
                || self.class_counts.get(mention) == Some(&1)
                || !self.scopes.iter().any(|scope| scope.contains(mention))
        })
    }

    fn enter(&mut self, bindings: ScopeBindings) {
        self.scopes.push(bindings.0);
    }
}

impl VisitMut for LocalClassLifter {
    fn visit_mut_function(&mut self, function: &mut swc_ecma_ast::Function) {
        let mut bindings = ScopeBindings::default();
        for parameter in &function.params {
            parameter.visit_with(&mut bindings);
        }
        if let Some(body) = &function.body {
            body.visit_with(&mut bindings);
        }
        self.enter(bindings);
        function.visit_mut_children_with(self);
        self.scopes.pop();
    }

    fn visit_mut_arrow_expr(&mut self, arrow: &mut swc_ecma_ast::ArrowExpr) {
        let mut bindings = ScopeBindings::default();
        for parameter in &arrow.params {
            parameter.visit_with(&mut bindings);
        }
        arrow.body.visit_with(&mut bindings);
        self.enter(bindings);
        arrow.visit_mut_children_with(self);
        self.scopes.pop();
    }

    fn visit_mut_constructor(&mut self, constructor: &mut swc_ecma_ast::Constructor) {
        let mut bindings = ScopeBindings::default();
        for parameter in &constructor.params {
            parameter.visit_with(&mut bindings);
        }
        if let Some(body) = &constructor.body {
            body.visit_with(&mut bindings);
        }
        self.enter(bindings);
        constructor.visit_mut_children_with(self);
        self.scopes.pop();
    }

    fn visit_mut_stmts(&mut self, statements: &mut Vec<Stmt>) {
        statements.visit_mut_children_with(self);
        let mut kept = Vec::with_capacity(statements.len());
        for statement in statements.drain(..) {
            match statement {
                Stmt::Decl(Decl::Class(declaration)) if self.liftable(&declaration) => {
                    self.lifted
                        .push(ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))));
                }
                other => kept.push(other),
            }
        }
        *statements = kept;
    }
}

/// Moves class declarations nested in function bodies/blocks to module level
/// when they capture no enclosing local (module-level names, their own
/// bindings and other lifted local classes are fine). Captures of locals are
/// not supported and keep the original statement (and its diagnostic).
fn lift_local_classes(module: &Module) -> Module {
    #[derive(Default)]
    struct LocalClassNames(HashMap<String, usize>);
    impl Visit for LocalClassNames {
        fn visit_stmts(&mut self, statements: &[Stmt]) {
            for statement in statements {
                if let Stmt::Decl(Decl::Class(declaration)) = statement {
                    *self.0.entry(declaration.ident.sym.to_string()).or_default() += 1;
                }
            }
            statements.visit_children_with(self);
        }
    }
    let mut counts = LocalClassNames::default();
    module.visit_with(&mut counts);
    if counts.0.is_empty() {
        return module.clone();
    }
    let mut top = ScopeBindings::default();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(statement) => statement.visit_with(&mut top),
            ModuleItem::ModuleDecl(declaration) => declaration.visit_with(&mut top),
        }
    }
    // Names declared at module level, plus everything imported or exported.
    let mut top_names = top.0;
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(declaration))) => {
                top_names.insert(declaration.id.sym.to_string());
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(declaration))) => {
                top_names.insert(declaration.id.sym.to_string());
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(declaration))) => {
                top_names.insert(declaration.id.sym.to_string());
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                let mut bindings = ScopeBindings::default();
                export.decl.visit_with(&mut bindings);
                top_names.extend(bindings.0);
            }
            _ => {}
        }
    }
    let mut lifter = LocalClassLifter {
        top_names,
        class_counts: counts.0,
        scopes: Vec::new(),
        lifted: Vec::new(),
    };
    let mut result = module.clone();
    result.visit_mut_with(&mut lifter);
    if lifter.lifted.is_empty() {
        return result;
    }
    let mut body = lifter.lifted;
    body.append(&mut result.body);
    result.body = body;
    result
}

struct LocalTypeLifter {
    top_names: HashSet<String>,
    counts: HashMap<String, usize>,
    lifted: Vec<ModuleItem>,
}

impl VisitMut for LocalTypeLifter {
    fn visit_mut_stmts(&mut self, statements: &mut Vec<Stmt>) {
        statements.visit_mut_children_with(self);
        let mut kept = Vec::with_capacity(statements.len());
        for statement in statements.drain(..) {
            let name = match &statement {
                Stmt::Decl(Decl::TsInterface(declaration)) => Some(declaration.id.sym.to_string()),
                Stmt::Decl(Decl::TsTypeAlias(declaration)) => Some(declaration.id.sym.to_string()),
                _ => None,
            };
            match name {
                Some(name) if !self.top_names.contains(&name) && self.counts.get(&name) == Some(&1) => {
                    self.lifted.push(ModuleItem::Stmt(statement));
                }
                _ => kept.push(statement),
            }
        }
        *statements = kept;
    }
}

/// Moves `type`/`interface` declarations nested in function bodies/blocks to module level. Types
/// are erased and resolved by name, so this is sound when the name is unique in the module; a
/// colliding name keeps the original statement (and its diagnostic).
fn lift_local_type_declarations(module: &Module) -> Module {
    #[derive(Default)]
    struct Names(HashMap<String, usize>);
    impl Visit for Names {
        fn visit_stmts(&mut self, statements: &[Stmt]) {
            for statement in statements {
                if let Stmt::Decl(Decl::TsInterface(d)) = statement {
                    *self.0.entry(d.id.sym.to_string()).or_default() += 1;
                }
                if let Stmt::Decl(Decl::TsTypeAlias(d)) = statement {
                    *self.0.entry(d.id.sym.to_string()).or_default() += 1;
                }
            }
            statements.visit_children_with(self);
        }
    }
    let mut names = Names::default();
    // Only declarations below the top level are candidates.
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))) => declaration.visit_with(&mut names),
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => declaration.visit_with(&mut names),
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) => declaration.visit_with(&mut names),
            ModuleItem::Stmt(Stmt::Expr(statement)) => statement.visit_with(&mut names),
            _ => {}
        }
    }
    if names.0.is_empty() {
        return module.clone();
    }
    let mut top_names = HashSet::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(d))) => { top_names.insert(d.id.sym.to_string()); }
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(d))) => { top_names.insert(d.id.sym.to_string()); }
            ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(d))) => { top_names.insert(d.id.sym.to_string()); }
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(d))) => { top_names.insert(d.ident.sym.to_string()); }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                let mut bindings = ScopeBindings::default();
                export.decl.visit_with(&mut bindings);
                top_names.extend(bindings.0);
                match &export.decl {
                    Decl::TsInterface(d) => { top_names.insert(d.id.sym.to_string()); }
                    Decl::TsTypeAlias(d) => { top_names.insert(d.id.sym.to_string()); }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let mut lifter = LocalTypeLifter { top_names, counts: names.0, lifted: Vec::new() };
    let mut result = module.clone();
    // Walk function bodies only (statement lists at the module top level are left alone).
    for item in &mut result.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))) => declaration.visit_mut_with(&mut lifter),
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => declaration.visit_mut_with(&mut lifter),
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) => declaration.visit_mut_with(&mut lifter),
            ModuleItem::Stmt(Stmt::Expr(statement)) => statement.visit_mut_with(&mut lifter),
            _ => {}
        }
    }
    if lifter.lifted.is_empty() {
        return result;
    }
    let mut body = lifter.lifted;
    body.append(&mut result.body);
    result.body = body;
    result
}
