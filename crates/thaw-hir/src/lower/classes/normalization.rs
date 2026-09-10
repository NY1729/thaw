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
    constants: &HashMap<Symbol, String>,
) -> Option<String> {
    match expression {
        Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
        Expr::Member(member)
            if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "Symbol")
                && matches!(&member.prop, MemberProp::Ident(property) if property.sym == "iterator") =>
        {
            Some("__thaw_symbol_iterator".into())
        }
        Expr::Ident(identifier) => constants.get(identifier.sym.as_ref()).cloned(),
        Expr::Bin(binary) if binary.op == BinaryOp::Add => Some(format!(
            "{}{}",
            static_class_member_name(&binary.left, constants)?,
            static_class_member_name(&binary.right, constants)?
        )),
        Expr::Paren(parenthesized) => static_class_member_name(&parenthesized.expr, constants),
        Expr::TsAs(assertion) => static_class_member_name(&assertion.expr, constants),
        Expr::TsTypeAssertion(assertion) => static_class_member_name(&assertion.expr, constants),
        Expr::TsConstAssertion(assertion) => static_class_member_name(&assertion.expr, constants),
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
                    value.push_str(&static_class_member_name(expression, constants)?);
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
            if let Some(value) = static_class_member_name(initializer, &constants) {
                constants.insert(name.clone(), value);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let normalize_class = |class: &mut swc_ecma_ast::Class| {
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
            let Some(value) = static_class_member_name(&computed.expr, &constants) else {
                continue;
            };
            *key = PropName::Str(swc_ecma_ast::Str {
                span,
                value: value.into(),
                raw: None,
            });
        }
    };
    let mut normalized = module.clone();
    for item in &mut normalized.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => {
                normalize_class(&mut declaration.class)
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                if let Decl::Class(declaration) = &mut export.decl {
                    normalize_class(&mut declaration.class);
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
            let Some(value) = static_class_member_name(&computed.expr, self.constants) else {
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
            let Some(value) = static_class_member_name(&computed.expr, self.constants) else {
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
    format!("__thaw_private_{owner}_{name}")
}

struct PrivateMemberNormalizer<'a> {
    owner: &'a str,
}

impl VisitMut for PrivateMemberNormalizer<'_> {
    fn visit_mut_expr(&mut self, expression: &mut Expr) {
        if let Expr::PrivateName(name) = expression {
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
    for item in &mut module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        let owner = declaration.ident.sym.to_string();
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
            .visit_mut_with(&mut PrivateMemberNormalizer { owner: &owner });
    }
    module
}
