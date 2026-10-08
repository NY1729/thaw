fn normalize_top_level_class_expressions(module: &Module) -> Result<Module, String> {
    let mut body = Vec::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) = item else {
            body.push(item.clone());
            continue;
        };
        for declarator in &declaration.decls {
            let Some(Expr::Class(expression)) = declarator.init.as_deref() else {
                let mut declaration = declaration.as_ref().clone();
                declaration.decls = vec![declarator.clone()];
                body.push(ModuleItem::Stmt(Stmt::Decl(Decl::Var(Box::new(
                    declaration,
                )))));
                continue;
            };
            let Pat::Ident(binding) = &declarator.name else {
                return Err("top-level class expressions require an identifier binding".into());
            };
            let mut class = expression.class.as_ref().clone();
            if let Some(internal) = &expression.ident {
                if internal.sym != binding.id.sym {
                    class.visit_mut_with(&mut ClassSelfReferenceRenamer {
                        from: internal.sym.as_ref(),
                        to: binding.id.sym.as_ref(),
                        shadowed: Vec::new(),
                    });
                }
            }
            body.push(ModuleItem::Stmt(Stmt::Decl(Decl::Class(ClassDecl {
                ident: binding.id.clone(),
                declare: false,
                class: Box::new(class),
            }))));
        }
    }
    Ok(Module {
        body,
        ..module.clone()
    })
}

struct ClassSelfReferenceRenamer<'a> {
    from: &'a str,
    to: &'a str,
    shadowed: Vec<bool>,
}

impl ClassSelfReferenceRenamer<'_> {
    fn is_shadowed(&self) -> bool {
        self.shadowed.iter().rev().any(|shadowed| *shadowed)
    }

    fn rename(&self, identifier: &mut swc_ecma_ast::Ident) {
        if !self.is_shadowed() && identifier.sym == *self.from {
            identifier.sym = self.to.into();
        }
    }
}

#[derive(Default)]
struct ClassSelfBindingCollector {
    names: HashSet<Symbol>,
}

impl Visit for ClassSelfBindingCollector {
    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        self.names.insert(binding.id.sym.to_string());
    }

    fn visit_function(&mut self, _function: &swc_ecma_ast::Function) {}

    fn visit_arrow_expr(&mut self, _arrow: &swc_ecma_ast::ArrowExpr) {}
}

impl VisitMut for ClassSelfReferenceRenamer<'_> {
    fn visit_mut_function(&mut self, function: &mut swc_ecma_ast::Function) {
        let mut bindings = ClassSelfBindingCollector::default();
        for parameter in &function.params {
            parameter.pat.visit_with(&mut bindings);
        }
        if let Some(body) = &function.body {
            body.visit_with(&mut bindings);
        }
        self.shadowed.push(bindings.names.contains(self.from));
        function.visit_mut_children_with(self);
        self.shadowed.pop();
    }

    fn visit_mut_arrow_expr(&mut self, arrow: &mut swc_ecma_ast::ArrowExpr) {
        let mut bindings = ClassSelfBindingCollector::default();
        for parameter in &arrow.params {
            parameter.visit_with(&mut bindings);
        }
        arrow.body.visit_with(&mut bindings);
        self.shadowed.push(bindings.names.contains(self.from));
        arrow.visit_mut_children_with(self);
        self.shadowed.pop();
    }

    fn visit_mut_class_decl(&mut self, declaration: &mut ClassDecl) {
        self.shadowed
            .push(declaration.ident.sym == *self.from);
        declaration.visit_mut_children_with(self);
        self.shadowed.pop();
    }

    fn visit_mut_class_expr(&mut self, expression: &mut swc_ecma_ast::ClassExpr) {
        self.shadowed.push(
            expression
                .ident
                .as_ref()
                .is_some_and(|identifier| identifier.sym == *self.from),
        );
        expression.visit_mut_children_with(self);
        self.shadowed.pop();
    }

    fn visit_mut_expr(&mut self, expression: &mut Expr) {
        expression.visit_mut_children_with(self);
        if let Expr::Ident(identifier) = expression {
            self.rename(identifier);
        }
    }

    fn visit_mut_ts_type_ref(&mut self, reference: &mut swc_ecma_ast::TsTypeRef) {
        reference.visit_mut_children_with(self);
        if let swc_ecma_ast::TsEntityName::Ident(identifier) = &mut reference.type_name {
            self.rename(identifier);
        }
    }
}

fn static_class_member_name(
    expression: &Expr,
    constant: &impl Fn(&str) -> Option<String>,
) -> Option<String> {
    match expression {
        Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
        Expr::Member(member)
            if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "Symbol")
                && matches!(&member.prop, MemberProp::Ident(property) if property.sym == "iterator") =>
        {
            Some("__thaw_symbol_iterator".into())
        }
        Expr::Ident(identifier) => constant(identifier.sym.as_ref()),
        Expr::Bin(binary) if binary.op == BinaryOp::Add => Some(format!(
            "{}{}",
            static_class_member_name(&binary.left, constant)?,
            static_class_member_name(&binary.right, constant)?
        )),
        Expr::Paren(parenthesized) => static_class_member_name(&parenthesized.expr, constant),
        Expr::TsAs(assertion) => static_class_member_name(&assertion.expr, constant),
        Expr::TsTypeAssertion(assertion) => static_class_member_name(&assertion.expr, constant),
        Expr::TsConstAssertion(assertion) => static_class_member_name(&assertion.expr, constant),
        Expr::Tpl(template) => {
            let mut value = String::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                let quasi = quasi
                    .cooked
                    .as_ref()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| quasi.raw.to_string());
                value.push_str(&quasi);
                if let Some(expression) = template.exprs.get(index) {
                    value.push_str(&static_class_member_name(expression, constant)?);
                }
            }
            Some(value)
        }
        _ => None,
    }
}

fn normalize_static_computed_class_members(module: &Module) -> Module {
    let mut declarations = Vec::new();
    for item in &module.body {
        let declaration = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration)))
                if declaration.kind == swc_ecma_ast::VarDeclKind::Const =>
            {
                Some(declaration.as_ref())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                let Decl::Var(declaration) = &export.decl else {
                    continue;
                };
                (declaration.kind == swc_ecma_ast::VarDeclKind::Const)
                    .then_some(declaration.as_ref())
            }
            _ => None,
        };
        let Some(declaration) = declaration else {
            continue;
        };
        for declarator in &declaration.decls {
            let (Pat::Ident(binding), Some(initializer)) =
                (&declarator.name, declarator.init.as_deref())
            else {
                continue;
            };
            declarations.push((binding.id.sym.to_string(), initializer));
        }
    }
    let mut constants = HashMap::new();
    loop {
        let mut changed = false;
        for (name, initializer) in &declarations {
            if constants.contains_key(name) {
                continue;
            }
            if let Some(value) =
                static_class_member_name(initializer, &|name| constants.get(name).cloned())
            {
                constants.insert(name.clone(), value);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let normalize_class = |class: &mut swc_ecma_ast::Class, initialized: &HashMap<Symbol, String>| {
        let mut uninitialized_key = None;
        for member in &mut class.body {
            let key = match member {
                ClassMember::ClassProp(property) => Some(&mut property.key),
                ClassMember::Method(method) => Some(&mut method.key),
                _ => None,
            };
            let Some(key) = key else {
                continue;
            };
            let PropName::Computed(computed) = key else {
                continue;
            };
            let span = computed.span;
            // `static_class_member_name` only special-cases `Symbol.
            // iterator` (its result is also used by the broader,
            // module-wide `ComputedAccessNormalizer` visitor below,
            // which must *not* eagerly rewrite every other well-known
            // symbol -- an object literal's own `[Symbol.toPrimitive]`
            // key, or a real `d[Symbol.toPrimitive]("number")` member
            // access, needs to still see the original `Symbol.<name>`
            // expression, not a pre-mangled string). This closure is
            // scoped to *class member declarations* only, though, so a
            // well-known-symbol fallback here -- mangled the same way
            // an object literal's own computed key is
            // (`well_known_symbol_key`, `objects.rs`), so
            // `invoke_class_to_primitive` finds it under the same
            // sentinel -- is safe and doesn't leak into that broader
            // pass.
            let missing = RefCell::new(None);
            let Some(value) = static_class_member_name(&computed.expr, &|name| {
                initialized.get(name).cloned().or_else(|| {
                    let value = constants.get(name).cloned()?;
                    if missing.borrow().is_none() {
                        *missing.borrow_mut() = Some(name.to_string());
                    }
                    Some(value)
                })
            })
            .or_else(|| well_known_symbol_from_expr(&computed.expr).map(well_known_symbol_key))
            else {
                continue;
            };
            if uninitialized_key.is_none() {
                uninitialized_key = missing.into_inner();
            }
            *key = PropName::Str(swc_ecma_ast::Str {
                span,
                value: value.into(),
                raw: None,
            });
        }
        if let Some(name) = uninitialized_key {
            let span = class.span;
            class.body.insert(0, ClassMember::StaticBlock(swc_ecma_ast::StaticBlock {
                span,
                body: swc_ecma_ast::BlockStmt {
                    span,
                    ctxt: Default::default(),
                    stmts: vec![Stmt::Throw(swc_ecma_ast::ThrowStmt {
                        span,
                        arg: Box::new(Expr::Call(swc_ecma_ast::CallExpr {
                            span,
                            ctxt: Default::default(),
                            callee: Callee::Expr(Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt("@@thaw_class_key_reference_error".into(), span)))),
                            args: vec![swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                                    span,
                                    value: format!("Cannot access '{name}' before initialization").into(),
                                    raw: None,
                                }))),
                            }],
                            type_args: None,
                        })),
                    })],
                },
            }));
        }
    };
    let mut normalized = module.clone();
    let mut initialized = HashMap::new();
    for item in &mut normalized.body {
        let variable = match &*item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(variable))) => Some(variable.as_ref()),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Var(variable) => Some(variable.as_ref()),
                _ => None,
            },
            _ => None,
        };
        if let Some(variable) = variable {
            if variable.kind == swc_ecma_ast::VarDeclKind::Const {
                for declarator in &variable.decls {
                    if let (Pat::Ident(binding), Some(value)) = (&declarator.name, declarator.init.as_deref()) {
                        if let Some(value) = static_class_member_name(value, &|name| initialized.get(name).cloned()) {
                            initialized.insert(binding.id.sym.to_string(), value);
                        }
                    }
                }
            }
        }
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => {
                normalize_class(&mut declaration.class, &initialized)
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                if let Decl::Class(declaration) = &mut export.decl {
                    normalize_class(&mut declaration.class, &initialized);
                }
            }
            _ => {}
        }
    }
    struct ComputedAccessNormalizer<'a> {
        constants: &'a HashMap<Symbol, String>,
        shadowed: Vec<HashSet<Symbol>>,
    }
    impl ComputedAccessNormalizer<'_> {
        fn local_constant_shadows<T: VisitWith<BindingCollector>>(
            &self,
            node: &T,
        ) -> HashSet<Symbol> {
            let mut collector = BindingCollector::default();
            node.visit_with(&mut collector);
            collector
                .names
                .into_iter()
                .filter(|name| self.constants.contains_key(name))
                .collect()
        }

        fn expression_is_shadowed(&self, expression: &Expr) -> bool {
            let mut collector = IdentifierCollector::default();
            expression.visit_with(&mut collector);
            collector.names.into_iter().any(|name| {
                self.shadowed
                    .iter()
                    .rev()
                    .any(|scope| scope.contains(&name))
            })
        }
    }
    #[derive(Default)]
    struct BindingCollector {
        names: HashSet<Symbol>,
    }
    impl Visit for BindingCollector {
        fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
            self.names.insert(binding.id.sym.to_string());
        }
    }
    #[derive(Default)]
    struct IdentifierCollector {
        names: HashSet<Symbol>,
    }
    impl Visit for IdentifierCollector {
        fn visit_ident(&mut self, identifier: &swc_ecma_ast::Ident) {
            self.names.insert(identifier.sym.to_string());
        }
    }
    impl VisitMut for ComputedAccessNormalizer<'_> {
        fn visit_mut_class_method(&mut self, method: &mut swc_ecma_ast::ClassMethod) {
            // Class keys were normalized using declaration-time constants.
            let key = method.key.clone();
            method.visit_mut_children_with(self);
            method.key = key;
        }

        fn visit_mut_class_prop(&mut self, property: &mut swc_ecma_ast::ClassProp) {
            let key = property.key.clone();
            property.visit_mut_children_with(self);
            property.key = key;
        }

        fn visit_mut_function(&mut self, function: &mut swc_ecma_ast::Function) {
            let shadowed = self.local_constant_shadows(function);
            self.shadowed.push(shadowed);
            function.visit_mut_children_with(self);
            self.shadowed.pop();
        }

        fn visit_mut_arrow_expr(&mut self, arrow: &mut swc_ecma_ast::ArrowExpr) {
            let shadowed = self.local_constant_shadows(arrow);
            self.shadowed.push(shadowed);
            arrow.visit_mut_children_with(self);
            self.shadowed.pop();
        }

        fn visit_mut_member_prop(&mut self, property: &mut MemberProp) {
            let MemberProp::Computed(computed) = property else {
                property.visit_mut_children_with(self);
                return;
            };
            if self.expression_is_shadowed(&computed.expr) {
                computed.visit_mut_children_with(self);
                return;
            }
            let Some(value) = static_class_member_name(&computed.expr, &|name| {
                self.constants.get(name).cloned()
            }) else {
                computed.visit_mut_children_with(self);
                return;
            };
            *computed.expr = Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                span: computed.span,
                value: value.into(),
                raw: None,
            }));
        }

        fn visit_mut_prop_name(&mut self, property: &mut PropName) {
            let PropName::Computed(computed) = property else {
                property.visit_mut_children_with(self);
                return;
            };
            if self.expression_is_shadowed(&computed.expr) {
                computed.visit_mut_children_with(self);
                return;
            }
            let Some(value) = static_class_member_name(&computed.expr, &|name| {
                self.constants.get(name).cloned()
            }) else {
                computed.visit_mut_children_with(self);
                return;
            };
            *property = PropName::Str(swc_ecma_ast::Str {
                span: computed.span,
                value: value.into(),
                raw: None,
            });
        }
    }
    normalized.visit_mut_with(&mut ComputedAccessNormalizer {
        constants: &constants,
        shadowed: Vec::new(),
    });
    normalized
}

fn private_member_name(owner: &str, name: &str) -> Symbol {
    PRIVATE_CLASS_SLOTS.with(|slots| slots.borrow().get(owner)
        .and_then(|fields| fields.get(name)).cloned())
        .unwrap_or_else(|| format!("__thaw_private_{owner}_{name}"))
}

struct PrivateMemberNormalizer<'a> {
    owner: &'a str,
    shadowed: Vec<HashSet<Symbol>>,
}

impl VisitMut for PrivateMemberNormalizer<'_> {
    fn visit_mut_class(&mut self, class: &mut swc_ecma_ast::Class) {
        // Heritage is evaluated in the enclosing private environment.
        let mut heritage = class.super_class.take();
        if let Some(expression) = &mut heritage { expression.visit_mut_with(self); }
        let names = class.body.iter().filter_map(|member| match member {
            ClassMember::PrivateProp(property) => Some(property.key.name.to_string()),
            ClassMember::PrivateMethod(method) => Some(method.key.name.to_string()),
            _ => None,
        }).collect();
        self.shadowed.push(names);
        class.visit_mut_children_with(self);
        self.shadowed.pop();
        class.super_class = heritage;
    }

    fn visit_mut_expr(&mut self, expression: &mut Expr) {
        if let Expr::PrivateName(name) = expression {
            if self.shadowed.iter().any(|scope| scope.contains(name.name.as_ref())) { return; }
            *expression = Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                span: name.span,
                value: private_member_name(self.owner, name.name.as_ref()).into(),
                raw: None,
            }));
            return;
        }
        expression.visit_mut_children_with(self);
    }

    fn visit_mut_member_prop(&mut self, property: &mut MemberProp) {
        if let MemberProp::PrivateName(name) = property {
            if self.shadowed.iter().any(|scope| scope.contains(name.name.as_ref())) { return; }
            *property = MemberProp::Ident(IdentName::new(
                private_member_name(self.owner, name.name.as_ref()).into(),
                name.span,
            ));
            return;
        }
        property.visit_mut_children_with(self);
    }
}

fn normalize_private_class_members(module: &Module) -> Module {
    let mut module = module.clone();
    // Reserve public names across the module before choosing internal slots:
    // an inherited public field can also collide with a derived private slot.
    let mut occupied = module.body.iter().filter_map(|item| match item {
        ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => Some(declaration),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::Class(declaration) => Some(declaration),
            _ => None,
        },
        _ => None,
    }).flat_map(|declaration| declaration.class.body.iter().filter_map(|member| {
        let key = match member {
            ClassMember::ClassProp(property) => &property.key,
            ClassMember::Method(method) => &method.key,
            _ => return None,
        };
        class_property_name(key).ok()
    })).collect::<HashSet<_>>();
    for item in &mut module.body {
        let declaration = match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => declaration,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &mut export.decl {
                Decl::Class(declaration) => declaration,
                _ => continue,
            },
            _ => continue,
        };
        let owner = declaration.ident.sym.to_string();
        let mut slots = HashMap::new();
        for member in &declaration.class.body {
            let name = match member {
                ClassMember::PrivateProp(property) => property.key.name.as_ref(),
                ClassMember::PrivateMethod(method) => method.key.name.as_ref(),
                _ => continue,
            };
            if slots.contains_key(name) { continue; }
            let mut slot = format!("__thaw_private_{owner}_{name}");
            while occupied.contains(&slot) { slot.push('_'); }
            occupied.insert(slot.clone());
            slots.insert(name.to_string(), slot);
        }
        PRIVATE_CLASS_SLOTS.with(|registry| { registry.borrow_mut().insert(owner.clone(), slots); });
        declaration.class.body = std::mem::take(&mut declaration.class.body)
            .into_iter()
            .map(|member| match member {
                ClassMember::PrivateProp(property) => ClassMember::ClassProp(ClassProp {
                    span: property.span,
                    key: PropName::Ident(IdentName::new(
                        private_member_name(&owner, property.key.name.as_ref()).into(),
                        property.key.span,
                    )),
                    value: property.value,
                    type_ann: property.type_ann,
                    is_static: property.is_static,
                    decorators: property.decorators,
                    accessibility: property.accessibility,
                    is_abstract: false,
                    is_optional: property.is_optional,
                    is_override: property.is_override,
                    readonly: property.readonly,
                    declare: false,
                    definite: property.definite,
                }),
                ClassMember::PrivateMethod(method) => ClassMember::Method(ClassMethod {
                    span: method.span,
                    key: PropName::Ident(IdentName::new(
                        private_member_name(&owner, method.key.name.as_ref()).into(),
                        method.key.span,
                    )),
                    function: method.function,
                    kind: method.kind,
                    is_static: method.is_static,
                    accessibility: method.accessibility,
                    is_abstract: method.is_abstract,
                    is_optional: method.is_optional,
                    is_override: method.is_override,
                }),
                other => other,
            })
            .collect();
        declaration
            .class
            .visit_mut_children_with(&mut PrivateMemberNormalizer { owner: &owner, shadowed: Vec::new() });
    }
    module
}
