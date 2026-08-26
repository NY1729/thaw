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

#[derive(Clone)]
struct GenericClassTemplate {
    declaration: ClassDecl,
    parameters: Vec<Symbol>,
    constraints: Vec<Option<Box<TsType>>>,
    defaults: Vec<Option<Box<TsType>>>,
    constructor_patterns: Vec<GenericTypePattern>,
}

fn generic_class_static_owner(name: &str) -> Symbol {
    format!("{name}__thaw_generic_static")
}

fn generic_class_static_member(member: &ClassMember) -> bool {
    match member {
        ClassMember::Method(method) => method.is_static,
        ClassMember::PrivateMethod(method) => method.is_static,
        ClassMember::ClassProp(property) => property.is_static,
        ClassMember::PrivateProp(property) => property.is_static,
        ClassMember::StaticBlock(_) => true,
        ClassMember::AutoAccessor(accessor) => accessor.is_static,
        _ => false,
    }
}

fn member_references_class_type_parameter(
    member: &ClassMember,
    parameters: &HashSet<Symbol>,
) -> Option<Symbol> {
    struct Detector<'a> {
        parameters: &'a HashSet<Symbol>,
        found: Option<Symbol>,
    }
    impl Visit for Detector<'_> {
        fn visit_ts_type_ref(&mut self, reference: &swc_ecma_ast::TsTypeRef) {
            if reference.type_params.is_none() {
                if let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name {
                    if self.parameters.contains(name.sym.as_ref()) {
                        self.found = Some(name.sym.to_string());
                        return;
                    }
                }
            }
            reference.visit_children_with(self);
        }
    }
    let mut detector = Detector {
        parameters,
        found: None,
    };
    member.visit_with(&mut detector);
    detector.found
}

struct GenericClassStaticReferenceRewriter<'a, 'ast> {
    owners: &'a HashMap<Symbol, Symbol>,
    templates: &'a HashMap<Symbol, GenericClassTemplate>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'ast>,
    error: Option<String>,
}

impl VisitMut for GenericClassStaticReferenceRewriter<'_, '_> {
    fn visit_mut_member_expr(&mut self, member: &mut MemberExpr) {
        if let Expr::TsInstantiation(instantiation) = member.obj.as_ref() {
            if let Expr::Ident(owner) = instantiation.expr.as_ref() {
                if let Some(shared) = self.owners.get(owner.sym.as_ref()) {
                    let arguments = unbox_types(&instantiation.type_args.params);
                    if let Err(error) = resolve_generic_class_type_tuple(
                        owner.sym.as_ref(),
                        &self.templates[owner.sym.as_ref()],
                        &arguments,
                        None,
                        self.interfaces,
                        self.generic_interfaces,
                    ) {
                        self.error = Some(error);
                        return;
                    }
                    *member.obj = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                        shared.clone().into(),
                        owner.span,
                    ));
                    member.prop.visit_mut_with(self);
                    return;
                }
            }
        }
        member.visit_mut_children_with(self);
        let Expr::Ident(owner) = member.obj.as_mut() else {
            return;
        };
        if let Some(shared) = self.owners.get(owner.sym.as_ref()) {
            owner.sym = shared.clone().into();
        }
    }
}

struct GenericClassUse {
    name: Symbol,
    arguments: Vec<TsType>,
    actual_params: Option<Vec<HirType>>,
    constructor_span: Option<swc_common::Span>,
}

struct GenericClassUseCollector<'a> {
    names: &'a HashSet<Symbol>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'a>,
    uses: Vec<GenericClassUse>,
    scopes: Vec<HashMap<Symbol, HirType>>,
    constructor_symbols: &'a HashMap<swc_common::Span, Symbol>,
    call_results: &'a HashMap<Symbol, HirType>,
}

fn unbox_types(types: &[Box<TsType>]) -> Vec<TsType> {
    types.iter().map(|ty| ty.as_ref().clone()).collect()
}

fn hir_type_as_ts_type(ty: &HirType) -> Result<TsType, String> {
    let keyword = |kind| {
        TsType::TsKeywordType(swc_ecma_ast::TsKeywordType {
            span: swc_common::DUMMY_SP,
            kind,
        })
    };
    Ok(match ty {
        HirType::F64 | HirType::I64 => keyword(TsKeywordTypeKind::TsNumberKeyword),
        HirType::Str => keyword(TsKeywordTypeKind::TsStringKeyword),
        HirType::Bool => keyword(TsKeywordTypeKind::TsBooleanKeyword),
        HirType::Void => keyword(TsKeywordTypeKind::TsVoidKeyword),
        HirType::Promise(result) => TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
            span: swc_common::DUMMY_SP,
            type_name: swc_ecma_ast::TsEntityName::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                "Promise".into(),
                swc_common::DUMMY_SP,
            )),
            type_params: Some(Box::new(swc_ecma_ast::TsTypeParamInstantiation {
                span: swc_common::DUMMY_SP,
                params: vec![Box::new(hir_type_as_ts_type(result)?)],
            })),
        }),
        HirType::Array(element) => TsType::TsArrayType(swc_ecma_ast::TsArrayType {
            span: swc_common::DUMMY_SP,
            elem_type: Box::new(hir_type_as_ts_type(element)?),
        }),
        HirType::Tuple(elements) => TsType::TsTupleType(swc_ecma_ast::TsTupleType {
            span: swc_common::DUMMY_SP,
            elem_types: elements
                .iter()
                .map(|element| {
                    Ok(swc_ecma_ast::TsTupleElement {
                        span: swc_common::DUMMY_SP,
                        label: None,
                        ty: Box::new(hir_type_as_ts_type(element)?),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        }),
        HirType::Object(fields) => TsType::TsTypeLit(swc_ecma_ast::TsTypeLit {
            span: swc_common::DUMMY_SP,
            members: fields
                .iter()
                .map(|(name, field)| {
                    Ok(TsTypeElement::TsPropertySignature(
                        swc_ecma_ast::TsPropertySignature {
                            span: swc_common::DUMMY_SP,
                            readonly: false,
                            key: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                                name.clone().into(),
                                swc_common::DUMMY_SP,
                            ))),
                            computed: false,
                            optional: false,
                            type_ann: Some(Box::new(swc_ecma_ast::TsTypeAnn {
                                span: swc_common::DUMMY_SP,
                                type_ann: Box::new(hir_type_as_ts_type(field)?),
                            })),
                        },
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?,
        }),
        other => {
            return Err(format!(
                "cannot express inferred generic class type {other:?}"
            ))
        }
    })
}

fn infer_generic_constructor_expr_type(
    expression: &Expr,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    scopes: &[HashMap<Symbol, HirType>],
    call_results: &HashMap<Symbol, HirType>,
) -> Result<HirType, String> {
    match expression {
        Expr::Lit(Lit::Num(_)) => Ok(HirType::F64),
        Expr::Lit(Lit::Str(_)) => Ok(HirType::Str),
        Expr::Tpl(template) if template.exprs.is_empty() => Ok(HirType::Str),
        Expr::Lit(Lit::Bool(_)) => Ok(HirType::Bool),
        Expr::Ident(identifier) => scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(identifier.sym.as_ref()).cloned())
            .ok_or_else(|| {
                format!(
                    "cannot infer a generic class type from identifier `{}`",
                    identifier.sym
                )
            }),
        Expr::Paren(parenthesized) => infer_generic_constructor_expr_type(
            &parenthesized.expr,
            interfaces,
            generic_interfaces,
            scopes,
            call_results,
        ),
        Expr::TsAs(assertion) => lower_ts_type(
            &assertion.type_ann,
            interfaces,
            generic_interfaces,
        ),
        Expr::TsTypeAssertion(assertion) => lower_ts_type(
            &assertion.type_ann,
            interfaces,
            generic_interfaces,
        ),
        Expr::Unary(unary) => match unary.op {
            UnaryOp::Bang => Ok(HirType::Bool),
            UnaryOp::TypeOf => Ok(HirType::Str),
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Tilde => Ok(HirType::F64),
            _ => Err("cannot infer this unary constructor argument type".into()),
        },
        Expr::Bin(binary) => {
            let left = infer_generic_constructor_expr_type(
                &binary.left,
                interfaces,
                generic_interfaces,
                scopes,
                call_results,
            )?;
            let right = infer_generic_constructor_expr_type(
                &binary.right,
                interfaces,
                generic_interfaces,
                scopes,
                call_results,
            )?;
            match binary.op {
                BinaryOp::EqEq
                | BinaryOp::NotEq
                | BinaryOp::EqEqEq
                | BinaryOp::NotEqEq
                | BinaryOp::Lt
                | BinaryOp::LtEq
                | BinaryOp::Gt
                | BinaryOp::GtEq
                | BinaryOp::In
                | BinaryOp::InstanceOf => Ok(HirType::Bool),
                BinaryOp::Add if left == HirType::Str || right == HirType::Str => {
                    Ok(HirType::Str)
                }
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::Exp
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitAnd
                | BinaryOp::LShift
                | BinaryOp::RShift
                | BinaryOp::ZeroFillRShift
                    if left == HirType::F64 && right == HirType::F64 =>
                {
                    Ok(HirType::F64)
                }
                BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::NullishCoalescing
                    if left == right =>
                {
                    Ok(left)
                }
                _ => Err(format!(
                    "cannot infer generic class type from binary operands {left:?} and {right:?}"
                )),
            }
        }
        Expr::Member(member) => {
            let receiver = infer_generic_constructor_expr_type(
                &member.obj,
                interfaces,
                generic_interfaces,
                scopes,
                call_results,
            )?;
            match (&receiver, &member.prop) {
                (HirType::Object(fields), MemberProp::Ident(property)) => fields
                    .iter()
                    .find_map(|(name, ty)| (name == property.sym.as_ref()).then(|| ty.clone()))
                    .ok_or_else(|| {
                        format!("inferred object has no field `{}`", property.sym)
                    }),
                (HirType::Array(_), MemberProp::Ident(property))
                    if property.sym == *"length" =>
                {
                    Ok(HirType::F64)
                }
                (HirType::Tuple(elements), MemberProp::Computed(property)) => {
                    let Expr::Lit(Lit::Num(index)) = property.expr.as_ref() else {
                        return Err("tuple inference requires a literal index".into());
                    };
                    elements
                        .get(index.value as usize)
                        .cloned()
                        .ok_or_else(|| "tuple inference index is out of bounds".into())
                }
                (HirType::Array(element), MemberProp::Computed(property))
                    if matches!(property.expr.as_ref(), Expr::Lit(Lit::Num(_))) =>
                {
                    Ok(element.as_ref().clone())
                }
                _ => Err("cannot infer this generic class member argument type".into()),
            }
        }
        Expr::Call(call) => {
            let Callee::Expr(callee) = &call.callee else {
                return Err("cannot infer a generic class type from this call target".into());
            };
            let Expr::Ident(callee) = callee.as_ref() else {
                let inferred = infer_generic_constructor_expr_type(
                    callee,
                    interfaces,
                    generic_interfaces,
                    scopes,
                    call_results,
                )?;
                let HirType::Function(_, result) = inferred else {
                    return Err("generic class call target is not a typed function".into());
                };
                return Ok(result.as_ref().clone());
            };
            if let Some(HirType::Function(_, result)) = scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(callee.sym.as_ref()))
            {
                return Ok(result.as_ref().clone());
            }
            match callee.sym.as_ref() {
                "String" => Ok(HirType::Str),
                "Number" => Ok(HirType::F64),
                "Boolean" => Ok(HirType::Bool),
                name => call_results.get(name).cloned().ok_or_else(|| {
                    format!("cannot infer the return type of call `{name}(...)`")
                }),
            }
        }
        Expr::Arrow(arrow) => {
            let parameters = arrow
                .params
                .iter()
                .map(|parameter| {
                    let Pat::Ident(binding) = parameter else {
                        return Err("call-result inference requires identifier arrow parameters".into());
                    };
                    let annotation = binding.type_ann.as_ref().ok_or_else(|| {
                        "call-result inference requires annotated arrow parameters".to_string()
                    })?;
                    lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                })
                .collect::<Result<Vec<_>, String>>()?;
            let result = if let Some(annotation) = arrow.return_type.as_ref() {
                lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?
            } else {
                match arrow.body.as_ref() {
                    ArrowFunctionBody::Expr(expression) => {
                        infer_generic_constructor_expr_type(
                            expression,
                            interfaces,
                            generic_interfaces,
                            scopes,
                            call_results,
                        )?
                    }
                    ArrowFunctionBody::FunctionBody(_) => {
                        return Err(
                            "block-bodied arrow call-result inference needs a return annotation"
                                .into(),
                        )
                    }
                }
            };
            Ok(HirType::Function(parameters, Box::new(result)))
        }
        Expr::Fn(function) => {
            let parameters = function
                .function
                .params
                .iter()
                .map(|parameter| {
                    let Pat::Ident(binding) = &parameter.pat else {
                        return Err("call-result inference requires identifier function parameters".into());
                    };
                    let annotation = binding.type_ann.as_ref().ok_or_else(|| {
                        "call-result inference requires annotated function parameters".to_string()
                    })?;
                    lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                })
                .collect::<Result<Vec<_>, String>>()?;
            let annotation = function.function.return_type.as_ref().ok_or_else(|| {
                "function-expression call-result inference needs a return annotation".to_string()
            })?;
            let result = lower_ts_type(
                &annotation.type_ann,
                interfaces,
                generic_interfaces,
            )?;
            Ok(HirType::Function(parameters, Box::new(result)))
        }
        Expr::Cond(conditional) => {
            let consequent = infer_generic_constructor_expr_type(
                &conditional.cons,
                interfaces,
                generic_interfaces,
                scopes,
                call_results,
            )?;
            let alternate = infer_generic_constructor_expr_type(
                &conditional.alt,
                interfaces,
                generic_interfaces,
                scopes,
                call_results,
            )?;
            if consequent == alternate {
                Ok(consequent)
            } else {
                Err(format!(
                    "conditional constructor argument has incompatible types {consequent:?} and {alternate:?}"
                ))
            }
        }
        Expr::Array(array) => {
            let mut elements = Vec::new();
            for element in &array.elems {
                let element = element
                    .as_ref()
                    .ok_or("cannot infer a generic class type from an array hole")?;
                if element.spread.is_some() {
                    return Err(
                        "cannot infer a generic class type from an array spread".into(),
                    );
                }
                elements.push(infer_generic_constructor_expr_type(
                    &element.expr,
                    interfaces,
                    generic_interfaces,
                    scopes,
                    call_results,
                )?);
            }
            let Some(first) = elements.first().cloned() else {
                return Err("cannot infer a generic class type from an empty array".into());
            };
            if elements.iter().all(|element| element == &first) {
                Ok(HirType::Array(Box::new(first)))
            } else {
                Ok(HirType::Tuple(elements))
            }
        }
        Expr::Object(object) => {
            let mut fields = Vec::new();
            for property in &object.props {
                let PropOrSpread::Prop(property) = property else {
                    return Err(
                        "cannot infer a generic class type from an object spread".into(),
                    );
                };
                if let Prop::Shorthand(identifier) = property.as_ref() {
                    fields.push((
                        identifier.sym.to_string(),
                        infer_generic_constructor_expr_type(
                            &Expr::Ident(identifier.clone()),
                            interfaces,
                            generic_interfaces,
                            scopes,
                            call_results,
                        )?,
                    ));
                    continue;
                }
                let Prop::KeyValue(property) = property.as_ref() else {
                    return Err(
                        "generic class object inference requires data properties".into(),
                    );
                };
                let name = match &property.key {
                    PropName::Ident(name) => name.sym.to_string(),
                    PropName::Str(name) => name.value.to_string_lossy().into_owned(),
                    _ => {
                        return Err(
                            "generic class object inference requires static property names".into(),
                        )
                    }
                };
                fields.push((
                    name,
                    infer_generic_constructor_expr_type(
                        &property.value,
                        interfaces,
                        generic_interfaces,
                        scopes,
                        call_results,
                    )?,
                ));
            }
            Ok(HirType::Object(fields))
        }
        _ => Err(
            "generic class constructor inference needs a literal, aggregate literal, or type assertion"
                .into(),
        ),
    }
}

fn generic_constructor_call_results(
    module: &Module,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> HashMap<Symbol, HirType> {
    struct ReturnTypes<'a, 'ast> {
        interfaces: &'a HashMap<Symbol, HirType>,
        generic_interfaces: &'a GenericInterfaces<'ast>,
        call_results: &'a HashMap<Symbol, HirType>,
        scopes: Vec<HashMap<Symbol, HirType>>,
        returned: Vec<HirType>,
        failed: bool,
    }
    impl Visit for ReturnTypes<'_, '_> {
        fn visit_return_stmt(&mut self, statement: &swc_ecma_ast::ReturnStmt) {
            if let Some(argument) = statement.arg.as_deref() {
                match infer_generic_constructor_expr_type(
                    argument,
                    self.interfaces,
                    self.generic_interfaces,
                    &self.scopes,
                    self.call_results,
                ) {
                    Ok(ty) => self.returned.push(ty),
                    Err(_) => self.failed = true,
                }
            }
        }

        fn visit_function(&mut self, _function: &swc_ecma_ast::Function) {}

        fn visit_arrow_expr(&mut self, _arrow: &swc_ecma_ast::ArrowExpr) {}

        fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
            self.scopes.push(HashMap::new());
            block.visit_children_with(self);
            self.scopes.pop();
        }

        fn visit_var_decl(&mut self, declaration: &swc_ecma_ast::VarDecl) {
            for declarator in &declaration.decls {
                let Pat::Ident(binding) = &declarator.name else {
                    continue;
                };
                let inferred = binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        lower_ts_type(
                            &annotation.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                    })
                    .or_else(|| {
                        declarator.init.as_deref().map(|initializer| {
                            infer_generic_constructor_expr_type(
                                initializer,
                                self.interfaces,
                                self.generic_interfaces,
                                &self.scopes,
                                self.call_results,
                            )
                        })
                    });
                if let Some(Ok(ty)) = inferred {
                    self.scopes
                        .last_mut()
                        .expect("return inference always has a scope")
                        .insert(binding.id.sym.to_string(), ty);
                }
            }
        }
    }

    let functions = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))) => Some(declaration),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut results = HashMap::new();
    for function in &functions {
        let Some(annotation) = function.function.return_type.as_ref() else {
            continue;
        };
        if let Ok(ty) = lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces) {
            results.insert(function.ident.sym.to_string(), ty);
        }
    }
    for _ in 0..=functions.len() {
        let mut changed = false;
        for function in &functions {
            if results.contains_key(function.ident.sym.as_ref()) {
                continue;
            }
            let Some(body) = function.function.body.as_ref() else {
                continue;
            };
            let mut scope = HashMap::new();
            let mut complete_params = true;
            for parameter in &function.function.params {
                let Pat::Ident(binding) = &parameter.pat else {
                    complete_params = false;
                    break;
                };
                let Some(annotation) = binding.type_ann.as_ref() else {
                    complete_params = false;
                    break;
                };
                let Ok(ty) = lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                else {
                    complete_params = false;
                    break;
                };
                scope.insert(binding.id.sym.to_string(), ty);
            }
            if !complete_params {
                continue;
            }
            let mut returned = ReturnTypes {
                interfaces,
                generic_interfaces,
                call_results: &results,
                scopes: vec![scope],
                returned: Vec::new(),
                failed: false,
            };
            body.visit_children_with(&mut returned);
            if returned.returned.is_empty() && !returned.failed {
                results.insert(function.ident.sym.to_string(), HirType::Void);
                changed = true;
                continue;
            }
            if returned.failed {
                continue;
            }
            let Some(first) = returned.returned.first() else {
                continue;
            };
            if returned.returned.iter().all(|candidate| candidate == first) {
                results.insert(function.ident.sym.to_string(), first.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    results
}

impl GenericClassUseCollector<'_> {
    fn bind_typed_pattern(&mut self, pattern: &Pat) {
        let Pat::Ident(binding) = pattern else {
            return;
        };
        let Some(annotation) = binding.type_ann.as_ref() else {
            return;
        };
        let Ok(ty) = lower_ts_type(
            &annotation.type_ann,
            self.interfaces,
            self.generic_interfaces,
        ) else {
            return;
        };
        self.scopes
            .last_mut()
            .expect("generic class collector always has a scope")
            .insert(binding.id.sym.to_string(), ty);
    }
}

impl Visit for GenericClassUseCollector<'_> {
    fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
        self.scopes.push(HashMap::new());
        for parameter in &function.params {
            self.bind_typed_pattern(&parameter.pat);
        }
        function.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        self.scopes.push(HashMap::new());
        for parameter in &arrow.params {
            self.bind_typed_pattern(parameter);
        }
        arrow.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
        self.scopes.push(HashMap::new());
        block.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_var_decl(&mut self, declaration: &swc_ecma_ast::VarDecl) {
        for declarator in &declaration.decls {
            declarator.name.visit_with(self);
            if let Some(initializer) = declarator.init.as_deref() {
                initializer.visit_with(self);
            }
            let Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            let inferred = binding
                .type_ann
                .as_ref()
                .map(|annotation| {
                    lower_ts_type(
                        &annotation.type_ann,
                        self.interfaces,
                        self.generic_interfaces,
                    )
                })
                .or_else(|| {
                    declarator.init.as_deref().and_then(|initializer| {
                        let Expr::New(construction) = initializer else {
                            return None;
                        };
                        self.constructor_symbols
                            .get(&construction.span)
                            .and_then(|symbol| self.interfaces.get(symbol))
                            .cloned()
                            .map(Ok)
                    })
                })
                .or_else(|| {
                    declarator.init.as_deref().map(|initializer| {
                        infer_generic_constructor_expr_type(
                            initializer,
                            self.interfaces,
                            self.generic_interfaces,
                            &self.scopes,
                            self.call_results,
                        )
                    })
                });
            if let Some(Ok(ty)) = inferred {
                self.scopes
                    .last_mut()
                    .expect("generic class collector always has a scope")
                    .insert(binding.id.sym.to_string(), ty);
            }
        }
    }

    fn visit_class(&mut self, class: &swc_ecma_ast::Class) {
        if let Some(Expr::Ident(super_class)) = class.super_class.as_deref() {
            if self.names.contains(super_class.sym.as_ref()) {
                self.uses.push(GenericClassUse {
                    name: super_class.sym.to_string(),
                    arguments: class
                        .super_type_params
                        .as_ref()
                        .map(|arguments| unbox_types(&arguments.params))
                        .unwrap_or_default(),
                    actual_params: None,
                    constructor_span: None,
                });
            }
        }
        class.visit_children_with(self);
    }

    fn visit_new_expr(&mut self, expression: &swc_ecma_ast::NewExpr) {
        if let Expr::Ident(class) = expression.callee.as_ref() {
            if self.names.contains(class.sym.as_ref()) {
                let inferred = if expression.type_args.is_none() {
                    expression.args.as_ref().map(|arguments| {
                        arguments
                            .iter()
                            .map(|argument| {
                                if argument.spread.is_some() {
                                    return Err("generic class constructor inference does not support spread arguments".into());
                                }
                                infer_generic_constructor_expr_type(
                                    &argument.expr,
                                    self.interfaces,
                                    self.generic_interfaces,
                                    &self.scopes,
                                    self.call_results,
                                )
                            })
                            .collect::<Result<Vec<_>, String>>()
                    })
                } else {
                    None
                };
                let inferred = match inferred.transpose() {
                    Ok(inferred) => inferred,
                    Err(error) => {
                        let _ = error;
                        None
                    }
                };
                self.uses.push(GenericClassUse {
                    name: class.sym.to_string(),
                    arguments: expression
                        .type_args
                        .as_ref()
                        .map(|arguments| unbox_types(&arguments.params))
                        .unwrap_or_default(),
                    actual_params: inferred,
                    constructor_span: Some(expression.span),
                });
            }
        }
        expression.visit_children_with(self);
    }

    fn visit_ts_type_ref(&mut self, reference: &swc_ecma_ast::TsTypeRef) {
        if let swc_ecma_ast::TsEntityName::Ident(class) = &reference.type_name {
            if self.names.contains(class.sym.as_ref()) {
                self.uses.push(GenericClassUse {
                    name: class.sym.to_string(),
                    arguments: reference
                        .type_params
                        .as_ref()
                        .map(|arguments| unbox_types(&arguments.params))
                        .unwrap_or_default(),
                    actual_params: None,
                    constructor_span: None,
                });
            }
        }
        reference.visit_children_with(self);
    }

    fn visit_ts_expr_with_type_args(&mut self, expression: &swc_ecma_ast::TsExprWithTypeArgs) {
        if let Expr::Ident(class) = expression.expr.as_ref() {
            if self.names.contains(class.sym.as_ref()) {
                self.uses.push(GenericClassUse {
                    name: class.sym.to_string(),
                    arguments: expression
                        .type_args
                        .as_ref()
                        .map(|arguments| unbox_types(&arguments.params))
                        .unwrap_or_default(),
                    actual_params: None,
                    constructor_span: None,
                });
            }
        }
        expression.visit_children_with(self);
    }
}

struct GenericClassTypeSubstituter<'a> {
    substitutions: &'a HashMap<Symbol, Box<TsType>>,
}

impl VisitMut for GenericClassTypeSubstituter<'_> {
    fn visit_mut_ts_type(&mut self, ty: &mut TsType) {
        if let TsType::TsTypeRef(reference) = ty {
            if reference.type_params.is_none() {
                if let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name {
                    if let Some(replacement) = self.substitutions.get(name.sym.as_ref()) {
                        *ty = replacement.as_ref().clone();
                        return;
                    }
                }
            }
        }
        ty.visit_mut_children_with(self);
    }
}

fn resolve_generic_class_type_tuple(
    name: &str,
    template: &GenericClassTemplate,
    arguments: &[TsType],
    actual_params: Option<&[HirType]>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<(Vec<HirType>, Vec<TsType>), String> {
    let required = template
        .defaults
        .iter()
        .filter(|default| default.is_none())
        .count();
    let infer_arguments = arguments.is_empty() && actual_params.is_some();
    if !infer_arguments
        && (arguments.len() < required || arguments.len() > template.parameters.len())
    {
        let expected = if required == template.parameters.len() {
            required.to_string()
        } else {
            format!("{required}..={}", template.parameters.len())
        };
        return Err(format!(
            "generic class `{name}` expects {expected} type argument(s), got {}",
            arguments.len()
        ));
    }

    let mut types = Vec::with_capacity(template.parameters.len());
    let mut concrete_arguments = Vec::with_capacity(template.parameters.len());
    let mut hir_substitution = HashMap::new();
    let mut ast_substitution = HashMap::new();
    let mut inferred = HashMap::new();
    if let Some(actual_params) = actual_params.filter(|_| infer_arguments) {
        for (pattern, actual) in template.constructor_patterns.iter().zip(actual_params) {
            match_generic_pattern(pattern, actual, &mut inferred)
                .map_err(|error| format!("cannot infer generic class `{name}`: {error}"))?;
        }
    }
    for (index, parameter) in template.parameters.iter().enumerate() {
        let (argument, concrete) = if let Some(argument) = arguments.get(index) {
            let mut argument = argument.clone();
            argument.visit_mut_with(&mut GenericClassTypeSubstituter {
                substitutions: &ast_substitution,
            });
            let concrete = lower_ts_type(&argument, interfaces, generic_interfaces)?;
            (argument, concrete)
        } else if let Some(inferred) = inferred.get(parameter) {
            (hir_type_as_ts_type(inferred)?, inferred.clone())
        } else {
            let mut argument = *template.defaults[index]
                .clone()
                .ok_or_else(|| {
                    format!(
                        "cannot infer generic class `{name}` type parameter `{parameter}` from its constructor arguments"
                    )
                })?;
            argument.visit_mut_with(&mut GenericClassTypeSubstituter {
                substitutions: &ast_substitution,
            });
            let concrete = lower_ts_type(&argument, interfaces, generic_interfaces)?;
            (argument, concrete)
        };
        ast_substitution.insert(parameter.clone(), Box::new(argument.clone()));
        hir_substitution.insert(parameter.clone(), concrete.clone());
        concrete_arguments.push(argument);
        types.push(concrete);
    }

    for ((parameter, actual), constraint) in template
        .parameters
        .iter()
        .zip(&types)
        .zip(&template.constraints)
    {
        let Some(constraint) = constraint else {
            continue;
        };
        let constraint = resolve_ts_type_with_substitution(
            constraint,
            &hir_substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?;
        if !type_satisfies_constraint(actual, &constraint) {
            return Err(format!(
                "generic class `{name}` type {actual:?} does not satisfy constraint {constraint:?} for `{parameter}`"
            ));
        }
    }
    Ok((types, concrete_arguments))
}

struct KnownGenericClassTypeRewriter<'a, 'ast> {
    specializations: &'a [(Symbol, Vec<HirType>, Symbol)],
    templates: &'a HashMap<Symbol, GenericClassTemplate>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'ast>,
}

impl KnownGenericClassTypeRewriter<'_, '_> {
    fn resolve(&self, name: &str, arguments: &[Box<TsType>]) -> Option<Symbol> {
        let template = self.templates.get(name)?;
        let arguments = unbox_types(arguments);
        let (types, _) = resolve_generic_class_type_tuple(
            name,
            template,
            &arguments,
            None,
            self.interfaces,
            self.generic_interfaces,
        )
        .ok()?;
        self.specializations
            .iter()
            .find_map(|(candidate, candidate_types, symbol)| {
                (candidate == name && candidate_types == &types).then(|| symbol.clone())
            })
    }
}

impl VisitMut for KnownGenericClassTypeRewriter<'_, '_> {
    fn visit_mut_ts_type_ref(&mut self, reference: &mut swc_ecma_ast::TsTypeRef) {
        reference.visit_mut_children_with(self);
        let Some(arguments) = reference.type_params.as_ref() else {
            return;
        };
        let swc_ecma_ast::TsEntityName::Ident(class) = &mut reference.type_name else {
            return;
        };
        if let Some(symbol) = self.resolve(class.sym.as_ref(), &arguments.params) {
            class.sym = symbol.into();
            reference.type_params = None;
        }
    }

    fn visit_mut_class(&mut self, class: &mut swc_ecma_ast::Class) {
        class.visit_mut_children_with(self);
        let Some(arguments) = class.super_type_params.as_ref() else {
            return;
        };
        let Some(Expr::Ident(base)) = class.super_class.as_deref_mut() else {
            return;
        };
        if let Some(symbol) = self.resolve(base.sym.as_ref(), &arguments.params) {
            base.sym = symbol.into();
            class.super_type_params = None;
        }
    }
}

fn rewrite_known_generic_class_types(
    arguments: &mut [TsType],
    specializations: &[(Symbol, Vec<HirType>, Symbol)],
    templates: &HashMap<Symbol, GenericClassTemplate>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) {
    let mut rewriter = KnownGenericClassTypeRewriter {
        specializations,
        templates,
        interfaces,
        generic_interfaces,
    };
    for argument in arguments {
        argument.visit_mut_with(&mut rewriter);
    }
}

fn class_contains_unresolved_generic_type(
    class: &swc_ecma_ast::Class,
    names: &HashSet<Symbol>,
) -> bool {
    struct Detector<'a> {
        names: &'a HashSet<Symbol>,
        found: bool,
    }
    impl Visit for Detector<'_> {
        fn visit_ts_type_ref(&mut self, reference: &swc_ecma_ast::TsTypeRef) {
            if let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name {
                if self.names.contains(name.sym.as_ref()) {
                    self.found = true;
                    return;
                }
            }
            reference.visit_children_with(self);
        }

        fn visit_class(&mut self, class: &swc_ecma_ast::Class) {
            if let Some(Expr::Ident(base)) = class.super_class.as_deref() {
                if self.names.contains(base.sym.as_ref()) {
                    self.found = true;
                    return;
                }
            }
            class.visit_children_with(self);
        }
    }
    let mut detector = Detector {
        names,
        found: false,
    };
    class.visit_with(&mut detector);
    detector.found
}

fn class_field_layout_cycle(module: &Module) -> Option<Vec<Symbol>> {
    struct References<'a> {
        classes: &'a HashSet<Symbol>,
        names: Vec<Symbol>,
    }
    impl Visit for References<'_> {
        fn visit_ts_type_ref(&mut self, reference: &swc_ecma_ast::TsTypeRef) {
            if let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name {
                if self.classes.contains(name.sym.as_ref()) {
                    self.names.push(name.sym.to_string());
                }
            }
            reference.visit_children_with(self);
        }
    }

    let declarations = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => {
                Some((declaration.ident.sym.to_string(), declaration))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let names = declarations.keys().cloned().collect::<HashSet<_>>();
    let mut edges = HashMap::<Symbol, Vec<Symbol>>::new();
    for (name, declaration) in &declarations {
        let mut references = References {
            classes: &names,
            names: Vec::new(),
        };
        for member in &declaration.class.body {
            match member {
                ClassMember::ClassProp(property) if !property.is_static => {
                    property.type_ann.visit_with(&mut references);
                }
                ClassMember::PrivateProp(property) if !property.is_static => {
                    property.type_ann.visit_with(&mut references);
                }
                ClassMember::Constructor(constructor) => {
                    for parameter in &constructor.params {
                        if matches!(parameter, ParamOrTsParamProp::TsParamProp(_)) {
                            class_constructor_param_pattern(parameter).visit_with(&mut references);
                        }
                    }
                }
                _ => {}
            }
        }
        references.names.sort();
        references.names.dedup();
        edges.insert(name.clone(), references.names);
    }

    fn visit(
        name: &str,
        edges: &HashMap<Symbol, Vec<Symbol>>,
        complete: &mut HashSet<Symbol>,
        active: &mut Vec<Symbol>,
    ) -> Option<Vec<Symbol>> {
        if let Some(index) = active.iter().position(|candidate| candidate == name) {
            let mut cycle = active[index..].to_vec();
            cycle.push(name.to_string());
            return Some(cycle);
        }
        if complete.contains(name) {
            return None;
        }
        active.push(name.to_string());
        for dependency in edges.get(name).into_iter().flatten() {
            if let Some(cycle) = visit(dependency, edges, complete, active) {
                return Some(cycle);
            }
        }
        active.pop();
        complete.insert(name.to_string());
        None
    }

    let mut complete = HashSet::new();
    for name in declarations.keys() {
        if let Some(cycle) = visit(name, &edges, &mut complete, &mut Vec::new()) {
            return Some(cycle);
        }
    }
    None
}

struct GenericClassReferenceRewriter<'a, 'ast> {
    specializations: &'a [(Symbol, Vec<HirType>, Symbol)],
    templates: &'a HashMap<Symbol, GenericClassTemplate>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'ast>,
    constructor_actuals: &'a HashMap<swc_common::Span, Vec<HirType>>,
    error: Option<String>,
}

impl GenericClassReferenceRewriter<'_, '_> {
    fn resolve(
        &mut self,
        name: &str,
        arguments: Option<&[Box<TsType>]>,
        actual_params: Option<&[HirType]>,
    ) -> Option<Symbol> {
        let template = self.templates.get(name)?;
        let arguments = arguments.map(unbox_types).unwrap_or_default();
        let types = match resolve_generic_class_type_tuple(
            name,
            template,
            &arguments,
            actual_params,
            self.interfaces,
            self.generic_interfaces,
        ) {
            Ok((types, _)) => types,
            Err(error) => {
                self.error = Some(error);
                return None;
            }
        };
        self.specializations
            .iter()
            .find_map(|(candidate, candidate_types, symbol)| {
                (candidate == name && candidate_types == &types).then(|| symbol.clone())
            })
    }
}

impl VisitMut for GenericClassReferenceRewriter<'_, '_> {
    fn visit_mut_class(&mut self, class: &mut swc_ecma_ast::Class) {
        class.visit_mut_children_with(self);
        let arguments = class
            .super_type_params
            .as_ref()
            .map(|arguments| arguments.params.clone());
        let Some(Expr::Ident(super_class)) = class.super_class.as_deref_mut() else {
            return;
        };
        let name = super_class.sym.to_string();
        if let Some(symbol) = self.resolve(&name, arguments.as_deref(), None) {
            super_class.sym = symbol.into();
            class.super_type_params = None;
        }
    }

    fn visit_mut_new_expr(&mut self, expression: &mut swc_ecma_ast::NewExpr) {
        expression.visit_mut_children_with(self);
        let Some(class_name) = expression
            .callee
            .as_ident()
            .map(|class| class.sym.to_string())
        else {
            return;
        };
        let Some(_template) = self.templates.get(&class_name) else {
            return;
        };
        let actual_params = self.constructor_actuals.get(&expression.span);
        let Expr::Ident(class) = expression.callee.as_mut() else {
            return;
        };
        if let Some(symbol) = self.resolve(
            class.sym.as_ref(),
            expression
                .type_args
                .as_ref()
                .map(|arguments| arguments.params.as_slice()),
            actual_params.map(Vec::as_slice),
        ) {
            class.sym = symbol.into();
            expression.type_args = None;
        }
    }

    fn visit_mut_ts_type_ref(&mut self, reference: &mut swc_ecma_ast::TsTypeRef) {
        reference.visit_mut_children_with(self);
        let swc_ecma_ast::TsEntityName::Ident(class) = &mut reference.type_name else {
            return;
        };
        if let Some(symbol) = self.resolve(
            class.sym.as_ref(),
            reference
                .type_params
                .as_ref()
                .map(|arguments| arguments.params.as_slice()),
            None,
        ) {
            class.sym = symbol.into();
            reference.type_params = None;
        }
    }

    fn visit_mut_ts_expr_with_type_args(
        &mut self,
        expression: &mut swc_ecma_ast::TsExprWithTypeArgs,
    ) {
        expression.visit_mut_children_with(self);
        let Expr::Ident(class) = expression.expr.as_mut() else {
            return;
        };
        if let Some(symbol) = self.resolve(
            class.sym.as_ref(),
            expression
                .type_args
                .as_ref()
                .map(|arguments| arguments.params.as_slice()),
            None,
        ) {
            class.sym = symbol.into();
            expression.type_args = None;
        }
    }
}

fn specialize_generic_classes(
    module: &Module,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Option<Module>, String> {
    let mut templates = HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        let Some(parameters) = declaration.class.type_params.as_ref() else {
            continue;
        };
        validate_trailing_type_parameter_defaults(
            "generic class",
            declaration.ident.sym.as_ref(),
            parameters,
        )?;
        let parameter_names = parameters
            .params
            .iter()
            .map(|parameter| parameter.name.sym.to_string())
            .collect::<Vec<_>>();
        let pattern_substitutions = parameter_names
            .iter()
            .map(|parameter| {
                (
                    parameter.clone(),
                    GenericTypePattern::Variable(parameter.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        let constructor_patterns = declaration
            .class
            .body
            .iter()
            .find_map(|member| match member {
                ClassMember::Constructor(constructor) => Some(&constructor.params),
                _ => None,
            })
            .map(|constructor_params| {
                constructor_params
                    .iter()
                    .map(|parameter| {
                        let pattern = class_constructor_param_pattern(parameter);
                        let Pat::Ident(binding) = pattern else {
                            return Err(format!(
                                "generic class `{}` constructor inference requires identifier parameters",
                                declaration.ident.sym
                            ));
                        };
                        let annotation = binding.type_ann.ok_or_else(|| {
                            format!(
                                "generic class `{}` constructor inference requires parameter annotations",
                                declaration.ident.sym
                            )
                        })?;
                        generic_type_pattern(
                            &annotation.type_ann,
                            &pattern_substitutions,
                            interfaces,
                            generic_interfaces,
                            &mut Vec::new(),
                        )
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .unwrap_or_default();
        templates.insert(
            declaration.ident.sym.to_string(),
            GenericClassTemplate {
                declaration: declaration.clone(),
                parameters: parameter_names,
                constraints: parameters
                    .params
                    .iter()
                    .map(|parameter| parameter.constraint.clone())
                    .collect(),
                defaults: parameters
                    .params
                    .iter()
                    .map(|parameter| parameter.default.clone())
                    .collect(),
                constructor_patterns,
            },
        );
    }
    if templates.is_empty() {
        return Ok(None);
    }

    let names = templates.keys().cloned().collect::<HashSet<_>>();
    let call_results = generic_constructor_call_results(module, interfaces, generic_interfaces);
    let declared_class_names = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => {
                Some(declaration.ident.sym.to_string())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut static_owners = HashMap::new();
    for (name, template) in &templates {
        if !template
            .declaration
            .class
            .body
            .iter()
            .any(generic_class_static_member)
        {
            continue;
        }
        let parameters = template.parameters.iter().cloned().collect::<HashSet<_>>();
        if let Some(parameter) = template
            .declaration
            .class
            .body
            .iter()
            .filter(|member| generic_class_static_member(member))
            .find_map(|member| member_references_class_type_parameter(member, &parameters))
        {
            return Err(format!(
                "generic class `{name}` static members cannot reference class type parameter `{parameter}`"
            ));
        }
        let owner = generic_class_static_owner(name);
        if declared_class_names.contains(&owner) {
            return Err(format!(
                "generic class `{name}` static owner `{owner}` conflicts with a class declaration"
            ));
        }
        static_owners.insert(name.clone(), owner);
    }
    let mut specialized = module.clone();
    for item in &mut specialized.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        let Some(owner) = static_owners.get(declaration.ident.sym.as_ref()) else {
            continue;
        };
        let shared_base = declaration
            .class
            .super_class
            .as_deref()
            .and_then(Expr::as_ident)
            .and_then(|base| static_owners.get(base.sym.as_ref()))
            .cloned();
        let generic_base = declaration
            .class
            .super_class
            .as_deref()
            .and_then(Expr::as_ident)
            .is_some_and(|base| templates.contains_key(base.sym.as_ref()));
        declaration.ident.sym = owner.clone().into();
        declaration.class.type_params = None;
        if let Some(shared_base) = shared_base {
            let span = declaration
                .class
                .super_class
                .as_deref()
                .and_then(Expr::as_ident)
                .map(|base| base.span)
                .unwrap_or(swc_common::DUMMY_SP);
            declaration.class.super_class = Some(Box::new(Expr::Ident(
                swc_ecma_ast::Ident::new_no_ctxt(shared_base.into(), span),
            )));
        } else if generic_base {
            declaration.class.super_class = None;
        }
        declaration.class.super_type_params = None;
        declaration.class.implements.clear();
        declaration.class.is_abstract = false;
        declaration.class.body.retain(generic_class_static_member);
    }
    specialized.body.retain(|item| {
        !matches!(item, ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration)))
            if templates.contains_key(declaration.ident.sym.as_ref()))
    });
    let mut instances: Vec<(Symbol, Vec<HirType>, Symbol)> = Vec::new();
    let mut constructor_actuals = HashMap::new();
    let mut constructor_symbols = HashMap::new();
    let mut specialization_interfaces = interfaces.clone();
    loop {
        let mut collector = GenericClassUseCollector {
            names: &names,
            interfaces: &specialization_interfaces,
            generic_interfaces,
            uses: Vec::new(),
            scopes: vec![HashMap::new()],
            constructor_symbols: &constructor_symbols,
            call_results: &call_results,
        };
        specialized.visit_with(&mut collector);
        let mut added = false;
        let mut deferred_error = None;
        for usage in collector.uses {
            let name = usage.name;
            let mut arguments = usage.arguments;
            let actual_params = usage.actual_params;
            let constructor_span = usage.constructor_span;
            let template = &templates[&name];
            rewrite_known_generic_class_types(
                &mut arguments,
                &instances,
                &templates,
                &specialization_interfaces,
                generic_interfaces,
            );
            let can_defer_inference = constructor_span.is_some() && arguments.is_empty();
            let resolved = resolve_generic_class_type_tuple(
                &name,
                template,
                &arguments,
                actual_params.as_deref(),
                &specialization_interfaces,
                generic_interfaces,
            );
            let (types, arguments) = match resolved {
                Ok(resolved) => resolved,
                Err(error)
                    if error.contains("generics are not supported yet")
                        || error.contains("cannot infer generic class")
                        || (can_defer_inference && error.contains("type argument(s), got 0")) =>
                {
                    deferred_error.get_or_insert(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            if let (Some(span), Some(actual_params)) = (constructor_span, actual_params.as_ref()) {
                constructor_actuals.insert(span, actual_params.clone());
            }
            if let Some(unsupported) = types.iter().find(|ty| !supports_generic_native_layout(ty)) {
                return Err(format!(
                    "generic class `{name}` cannot specialize for native layout {unsupported:?}"
                ));
            }
            let existing = instances
                .iter()
                .find_map(|(candidate, candidate_types, symbol)| {
                    (candidate == &name && candidate_types == &types).then(|| symbol.clone())
                });
            let symbol = existing
                .clone()
                .unwrap_or_else(|| specialized_generic_name(&name, &types));
            if let Some(span) = constructor_span {
                constructor_symbols.insert(span, symbol.clone());
            }
            if existing.is_some() {
                continue;
            }
            let substitutions = template
                .parameters
                .iter()
                .cloned()
                .zip(arguments.iter().cloned().map(Box::new))
                .collect::<HashMap<_, _>>();
            let mut declaration = template.declaration.clone();
            declaration.ident.sym = symbol.clone().into();
            declaration.class.type_params = None;
            declaration
                .class
                .body
                .retain(|member| !generic_class_static_member(member));
            declaration
                .class
                .visit_mut_with(&mut GenericClassTypeSubstituter {
                    substitutions: &substitutions,
                });
            specialized
                .body
                .push(ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))));
            instances.push((name, types, symbol));
            added = true;
        }
        if !added {
            if let Some(error) = deferred_error {
                return Err(format!(
                    "cannot resolve nested generic class specialization: {error}"
                ));
            }
            break;
        }
        let mut layout_module = specialized.clone();
        layout_module.visit_mut_with(&mut KnownGenericClassTypeRewriter {
            specializations: &instances,
            templates: &templates,
            interfaces: &specialization_interfaces,
            generic_interfaces,
        });
        layout_module.body.retain(|item| {
            !matches!(
                item,
                ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration)))
                    if declaration.class.super_type_params.is_some()
                        || class_contains_unresolved_generic_type(&declaration.class, &names)
            )
        });
        let mut updated_interfaces = interfaces.clone();
        collect_native_classes(&layout_module, &mut updated_interfaces, generic_interfaces)
            .map_err(|error| {
                if error.contains("unsupported type reference")
                    || error.contains("unknown type reference")
                {
                    format!(
                        "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
                    )
                } else {
                    error
                }
            })?;
        specialization_interfaces = updated_interfaces;
    }

    let mut rewriter = GenericClassReferenceRewriter {
        specializations: &instances,
        templates: &templates,
        interfaces: &specialization_interfaces,
        generic_interfaces,
        constructor_actuals: &constructor_actuals,
        error: None,
    };
    specialized.visit_mut_with(&mut rewriter);
    if let Some(error) = rewriter.error {
        if error.contains("generics are not supported yet") {
            return Err(format!(
                "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
            ));
        }
        return Err(error);
    }
    let mut static_rewriter = GenericClassStaticReferenceRewriter {
        owners: &static_owners,
        templates: &templates,
        interfaces: &specialization_interfaces,
        generic_interfaces,
        error: None,
    };
    specialized.visit_mut_with(&mut static_rewriter);
    if let Some(error) = static_rewriter.error {
        return Err(error);
    }
    if specialized.body.iter().any(|item| {
        matches!(
            item,
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration)))
                if class_contains_unresolved_generic_type(&declaration.class, &names)
        )
    }) {
        return Err(
            "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout"
                .into(),
        );
    }
    if let Some(cycle) = class_field_layout_cycle(&specialized) {
        return Err(format!(
            "recursive generic class field layout `{}` cannot use Thaw's fixed-size native layout",
            cycle.join(" -> ")
        ));
    }
    for item in &specialized.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        if !declaration.ident.sym.contains("__thaw_") {
            continue;
        }
        let validate = |annotation: &TsType| {
            lower_ts_type(annotation, &specialization_interfaces, generic_interfaces).map_err(
                |error| {
                    format!(
                        "recursive or unresolved generic class field layout in `{}` cannot use Thaw's fixed-size native layout: {error}",
                        declaration.ident.sym
                    )
                },
            )
        };
        for member in &declaration.class.body {
            match member {
                ClassMember::ClassProp(property) if !property.is_static => {
                    if let Some(annotation) = property.type_ann.as_ref() {
                        validate(&annotation.type_ann)?;
                    }
                }
                ClassMember::PrivateProp(property) if !property.is_static => {
                    if let Some(annotation) = property.type_ann.as_ref() {
                        validate(&annotation.type_ann)?;
                    }
                }
                ClassMember::Constructor(constructor) => {
                    for parameter in &constructor.params {
                        if !matches!(parameter, ParamOrTsParamProp::TsParamProp(_)) {
                            continue;
                        }
                        let Pat::Ident(binding) = class_constructor_param_pattern(parameter) else {
                            continue;
                        };
                        if let Some(annotation) = binding.type_ann {
                            validate(&annotation.type_ann)?;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(Some(specialized))
}

#[derive(Clone)]
struct GenericClassMethodTemplate {
    method: ClassMethod,
    parameters: Vec<Symbol>,
    constraints: Vec<Option<Box<TsType>>>,
    defaults: Vec<Option<Box<TsType>>>,
    parameter_patterns: Vec<GenericTypePattern>,
    rest_pattern: Option<GenericTypePattern>,
}

struct GenericClassMethodUse {
    span: swc_common::Span,
    class: Symbol,
    method: Symbol,
    arguments: Option<Vec<TsType>>,
    actual_params: Option<Vec<HirType>>,
}

struct GenericClassMethodUseCollector<'a, 'ast> {
    templates: &'a HashMap<(Symbol, Symbol), GenericClassMethodTemplate>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'ast>,
    scopes: Vec<HashMap<Symbol, HirType>>,
    uses: Vec<GenericClassMethodUse>,
    call_results: &'a HashMap<Symbol, HirType>,
    parents: &'a HashMap<Symbol, Symbol>,
    static_member_types: &'a HashMap<(Symbol, Symbol), HirType>,
    instance_member_types: &'a HashMap<(Symbol, Symbol), HirType>,
    instance_method_results: &'a HashMap<(Symbol, Symbol), HirType>,
    classes: &'a HashSet<Symbol>,
    current_classes: Vec<Symbol>,
    current_static_contexts: Vec<bool>,
    error: Option<String>,
}

impl GenericClassMethodUseCollector<'_, '_> {
    fn static_member_type(&self, class: &str, member: &str) -> Option<HirType> {
        let mut current = Some(class);
        while let Some(class) = current {
            if let Some(ty) = self
                .static_member_types
                .get(&(class.to_string(), member.to_string()))
            {
                return Some(ty.clone());
            }
            current = self.parents.get(class).map(String::as_str);
        }
        None
    }

    fn instance_member_type(
        &self,
        class: &str,
        member: &str,
        method_result: bool,
    ) -> Option<HirType> {
        let types = if method_result {
            self.instance_method_results
        } else {
            self.instance_member_types
        };
        let mut current = Some(class);
        while let Some(class) = current {
            if let Some(ty) = types.get(&(class.to_string(), member.to_string())) {
                return Some(ty.clone());
            }
            current = self.parents.get(class).map(String::as_str);
        }
        None
    }

    fn is_static_receiver(&self, expression: &Expr) -> bool {
        match expression {
            Expr::Ident(identifier) => self.classes.contains(identifier.sym.as_ref()),
            Expr::This(_) => self.current_static_contexts.last().copied().unwrap_or(true),
            Expr::Paren(parenthesized) => self.is_static_receiver(&parenthesized.expr),
            Expr::TsAs(assertion) => self.is_static_receiver(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => self.is_static_receiver(&assertion.expr),
            _ => false,
        }
    }

    fn infer_actual_type(&self, expression: &Expr) -> Result<HirType, String> {
        if let Expr::Member(member) = expression {
            if let Some(property) = member_property_name(&member.prop) {
                let class = self.receiver_class(&member.obj);
                let ty = class.as_deref().and_then(|class| {
                    if self.is_static_receiver(&member.obj) {
                        self.static_member_type(class, &property)
                    } else {
                        self.instance_member_type(class, &property, false)
                    }
                });
                if let Some(ty) = ty {
                    return Ok(ty);
                }
            }
        }
        if let Expr::Call(call) = expression {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = callee.as_ref() {
                    if let Some(method) = member_property_name(&member.prop) {
                        let class = self.receiver_class(&member.obj);
                        let ty = class.as_deref().and_then(|class| {
                            if self.is_static_receiver(&member.obj) {
                                self.static_member_type(class, &method)
                            } else {
                                self.instance_member_type(class, &method, true)
                            }
                        });
                        if let Some(ty) = ty {
                            return Ok(ty);
                        }
                    }
                }
            }
        }
        infer_generic_constructor_expr_type(
            expression,
            self.interfaces,
            self.generic_interfaces,
            &self.scopes,
            self.call_results,
        )
    }

    fn template_owner(&self, class: &str, method: &str) -> Option<Symbol> {
        let mut current = Some(class);
        while let Some(class) = current {
            if self
                .templates
                .contains_key(&(class.to_string(), method.to_string()))
            {
                return Some(class.to_string());
            }
            current = self.parents.get(class).map(String::as_str);
        }
        None
    }

    fn bind_pattern(&mut self, pattern: &Pat) {
        let Pat::Ident(binding) = pattern else {
            return;
        };
        let Some(annotation) = binding.type_ann.as_ref() else {
            return;
        };
        if let Ok(ty) = lower_ts_type(
            &annotation.type_ann,
            self.interfaces,
            self.generic_interfaces,
        ) {
            self.scopes
                .last_mut()
                .expect("generic method collector always has a scope")
                .insert(binding.id.sym.to_string(), ty);
        }
    }

    fn receiver_class(&self, expression: &Expr) -> Option<Symbol> {
        match expression {
            Expr::Ident(identifier) => {
                if self
                    .templates
                    .keys()
                    .any(|(class, _)| class == identifier.sym.as_ref())
                {
                    return Some(identifier.sym.to_string());
                }
                self.scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(identifier.sym.as_ref()))
                    .and_then(class_name_from_type)
                    .map(str::to_owned)
            }
            Expr::New(construction) => construction.callee.as_ident().and_then(|class| {
                self.interfaces
                    .get(class.sym.as_ref())
                    .and_then(class_name_from_type)
                    .map(str::to_owned)
            }),
            Expr::This(_) => self.current_classes.last().cloned(),
            Expr::Paren(parenthesized) => self.receiver_class(&parenthesized.expr),
            Expr::TsAs(assertion) => self.receiver_class(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => self.receiver_class(&assertion.expr),
            _ => infer_generic_constructor_expr_type(
                expression,
                self.interfaces,
                self.generic_interfaces,
                &self.scopes,
                self.call_results,
            )
            .ok()
            .as_ref()
            .and_then(class_name_from_type)
            .map(str::to_owned),
        }
    }

    fn call_actual_params(&self, call: &CallExpr) -> Result<Vec<HirType>, String> {
        let mut actual = Vec::new();
        for argument in &call.args {
            let ty = self.infer_actual_type(&argument.expr)?;
            if argument.spread.is_none() {
                actual.push(ty);
                continue;
            }
            let HirType::Tuple(elements) = ty else {
                return Err(format!(
                    "generic class method inference requires a statically sized tuple spread, got {ty:?}"
                ));
            };
            actual.extend(elements);
        }
        Ok(actual)
    }
}

impl Visit for GenericClassMethodUseCollector<'_, '_> {
    fn visit_class_decl(&mut self, declaration: &ClassDecl) {
        self.current_classes.push(declaration.ident.sym.to_string());
        declaration.class.visit_with(self);
        self.current_classes.pop();
    }

    fn visit_class_method(&mut self, method: &ClassMethod) {
        if method.function.type_params.is_some() {
            return;
        }
        self.current_static_contexts.push(method.is_static);
        method.visit_children_with(self);
        self.current_static_contexts.pop();
    }

    fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
        self.scopes.push(HashMap::new());
        for parameter in &function.params {
            self.bind_pattern(&parameter.pat);
        }
        function.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        self.scopes.push(HashMap::new());
        for parameter in &arrow.params {
            self.bind_pattern(parameter);
        }
        arrow.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
        self.scopes.push(HashMap::new());
        block.visit_children_with(self);
        self.scopes.pop();
    }

    fn visit_var_decl(&mut self, declaration: &swc_ecma_ast::VarDecl) {
        for declarator in &declaration.decls {
            declarator.name.visit_with(self);
            if let Some(initializer) = declarator.init.as_deref() {
                initializer.visit_with(self);
            }
            let Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            let ty = binding
                .type_ann
                .as_ref()
                .and_then(|annotation| {
                    lower_ts_type(
                        &annotation.type_ann,
                        self.interfaces,
                        self.generic_interfaces,
                    )
                    .ok()
                })
                .or_else(|| {
                    let initializer = declarator.init.as_deref()?;
                    match initializer {
                        Expr::New(construction) => {
                            let class = construction.callee.as_ident()?;
                            self.interfaces.get(class.sym.as_ref()).cloned()
                        }
                        Expr::Ident(identifier) => self
                            .scopes
                            .iter()
                            .rev()
                            .find_map(|scope| scope.get(identifier.sym.as_ref()).cloned()),
                        _ => None,
                    }
                });
            if let Some(ty) = ty {
                self.scopes
                    .last_mut()
                    .expect("generic method collector always has a scope")
                    .insert(binding.id.sym.to_string(), ty);
            }
        }
    }

    fn visit_call_expr(&mut self, call: &CallExpr) {
        if let Callee::Expr(callee) = &call.callee {
            if let Expr::Member(member) = callee.as_ref() {
                if let (Some(class), Some(method)) = (
                    self.receiver_class(&member.obj),
                    member_property_name(&member.prop),
                ) {
                    if let Some(owner) = self.template_owner(&class, &method) {
                        let actual_params = if call.type_args.is_none() {
                            match self.call_actual_params(call) {
                                Ok(actual) => Some(actual),
                                Err(error) => {
                                    self.error = Some(format!(
                                        "cannot infer generic method `{owner}.{method}`: {error}"
                                    ));
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        self.uses.push(GenericClassMethodUse {
                            span: call.span,
                            class: owner,
                            method,
                            arguments: call
                                .type_args
                                .as_ref()
                                .map(|arguments| unbox_types(&arguments.params)),
                            actual_params,
                        });
                    }
                }
            }
            if let Expr::SuperProp(member) = callee.as_ref() {
                if let Ok(method) = super_property_name(&member.prop) {
                    let owner = self
                        .current_classes
                        .last()
                        .and_then(|class| self.parents.get(class))
                        .and_then(|parent| self.template_owner(parent, &method));
                    if let Some(owner) = owner {
                        let actual_params = if call.type_args.is_none() {
                            match self.call_actual_params(call) {
                                Ok(actual) => Some(actual),
                                Err(error) => {
                                    self.error = Some(format!(
                                        "cannot infer generic super method `{owner}.{method}`: {error}"
                                    ));
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        self.uses.push(GenericClassMethodUse {
                            span: call.span,
                            class: owner,
                            method,
                            arguments: call
                                .type_args
                                .as_ref()
                                .map(|arguments| unbox_types(&arguments.params)),
                            actual_params,
                        });
                    }
                }
            }
        }
        call.visit_children_with(self);
    }

    fn visit_ts_instantiation(&mut self, instantiation: &swc_ecma_ast::TsInstantiation) {
        if let Expr::Member(member) = instantiation.expr.as_ref() {
            if let (Some(class), Some(method)) = (
                self.receiver_class(&member.obj),
                member_property_name(&member.prop),
            ) {
                if let Some(owner) = self.template_owner(&class, &method) {
                    self.uses.push(GenericClassMethodUse {
                        span: instantiation.span,
                        class: owner,
                        method,
                        arguments: Some(unbox_types(&instantiation.type_args.params)),
                        actual_params: None,
                    });
                    return;
                }
            }
        }
        instantiation.visit_children_with(self);
    }
}

fn resolve_explicit_generic_class_method_types(
    class: &str,
    method: &str,
    template: &GenericClassMethodTemplate,
    arguments: Option<&[TsType]>,
    actual_params: Option<&[HirType]>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<(Vec<HirType>, Vec<TsType>), String> {
    let required = template
        .defaults
        .iter()
        .filter(|default| default.is_none())
        .count();
    let explicit_count = arguments.map(<[TsType]>::len);
    if explicit_count.is_some_and(|count| count < required || count > template.parameters.len()) {
        let expected = if required == template.parameters.len() {
            required.to_string()
        } else {
            format!("{required}..={}", template.parameters.len())
        };
        return Err(format!(
            "generic method `{class}.{method}` expects {expected} type argument(s), got {}",
            explicit_count.unwrap()
        ));
    }
    let mut types = Vec::with_capacity(template.parameters.len());
    let mut concrete_arguments = Vec::with_capacity(template.parameters.len());
    let mut hir_substitution = HashMap::new();
    let mut ast_substitution = HashMap::new();
    let mut inferred = HashMap::new();
    if let Some(actual_params) = actual_params.filter(|_| arguments.is_none()) {
        let fixed_count = template
            .parameter_patterns
            .len()
            .saturating_sub(usize::from(template.rest_pattern.is_some()));
        for (pattern, actual) in template
            .parameter_patterns
            .iter()
            .take(fixed_count)
            .zip(actual_params)
        {
            match_generic_pattern(pattern, actual, &mut inferred).map_err(|error| {
                format!("cannot infer generic method `{class}.{method}`: {error}")
            })?;
        }
        if let Some(rest_pattern) = template.rest_pattern.as_ref() {
            for actual in actual_params.iter().skip(fixed_count) {
                match_generic_pattern(rest_pattern, actual, &mut inferred).map_err(|error| {
                    format!(
                        "cannot infer generic method `{class}.{method}` rest arguments: {error}"
                    )
                })?;
            }
        }
    }
    for (index, parameter) in template.parameters.iter().enumerate() {
        let mut argument = if let Some(argument) = arguments.and_then(|values| values.get(index)) {
            argument.clone()
        } else if let Some(inferred) = inferred.get(parameter) {
            hir_type_as_ts_type(inferred)?
        } else {
            template.defaults[index]
                .as_deref()
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "cannot infer generic method `{class}.{method}` type parameter `{parameter}` from its arguments"
                    )
                })?
        };
        argument.visit_mut_with(&mut GenericClassTypeSubstituter {
            substitutions: &ast_substitution,
        });
        let ty = lower_ts_type(&argument, interfaces, generic_interfaces)?;
        ast_substitution.insert(parameter.clone(), Box::new(argument.clone()));
        hir_substitution.insert(parameter.clone(), ty.clone());
        concrete_arguments.push(argument);
        types.push(ty);
    }
    for ((parameter, actual), constraint) in template
        .parameters
        .iter()
        .zip(&types)
        .zip(&template.constraints)
    {
        let Some(constraint) = constraint else {
            continue;
        };
        let constraint = resolve_ts_type_with_substitution(
            constraint,
            &hir_substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?;
        if !type_satisfies_constraint(actual, &constraint) {
            return Err(format!(
                "generic method `{class}.{method}` type {actual:?} does not satisfy constraint {constraint:?} for `{parameter}`"
            ));
        }
    }
    Ok((types, concrete_arguments))
}

struct GenericClassMethodCallRewriter<'a> {
    calls: &'a HashMap<swc_common::Span, Symbol>,
}

impl VisitMut for GenericClassMethodCallRewriter<'_> {
    fn visit_mut_expr(&mut self, expression: &mut Expr) {
        expression.visit_mut_children_with(self);
        let Expr::TsInstantiation(instantiation) = expression else {
            return;
        };
        let Some(method) = self.calls.get(&instantiation.span) else {
            return;
        };
        let Expr::Member(mut member) = instantiation.expr.as_ref().clone() else {
            return;
        };
        member.prop = MemberProp::Ident(IdentName::new(method.clone().into(), instantiation.span));
        *expression = Expr::Member(member);
    }

    fn visit_mut_call_expr(&mut self, call: &mut CallExpr) {
        call.visit_mut_children_with(self);
        let Some(method) = self.calls.get(&call.span) else {
            return;
        };
        let Callee::Expr(callee) = &mut call.callee else {
            return;
        };
        match callee.as_mut() {
            Expr::Member(member) => {
                member.prop = MemberProp::Ident(IdentName::new(method.clone().into(), call.span));
            }
            Expr::SuperProp(member) => {
                member.prop = SuperProp::Ident(IdentName::new(method.clone().into(), call.span));
            }
            _ => return,
        }
        call.type_args = None;
    }
}

fn generic_class_method_shape(
    class: &str,
    method_name: &str,
    method: &ClassMethod,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Option<GenericClassMethodShape>, String> {
    let Some(type_parameters) = method.function.type_params.as_ref() else {
        return Ok(None);
    };
    let substitutions = type_parameters
        .params
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            (
                parameter.name.sym.to_string(),
                GenericTypePattern::Variable(format!("__thaw_method_type_{index}")),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut parameters = Vec::new();
    let mut optional = Vec::new();
    for parameter in &method.function.params {
        let (annotation, is_optional) = match &parameter.pat {
            Pat::Ident(binding) => (binding.type_ann.as_ref(), binding.id.optional),
            Pat::Assign(assignment) => match assignment.left.as_ref() {
                Pat::Ident(binding) => (binding.type_ann.as_ref(), true),
                _ => (None, true),
            },
            Pat::Rest(rest) => (rest.type_ann.as_ref(), false),
            _ => (None, false),
        };
        let annotation = annotation.ok_or_else(|| {
            format!("generic method `{class}.{method_name}` needs annotated parameters")
        })?;
        parameters.push(generic_type_pattern(
            &annotation.type_ann,
            &substitutions,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?);
        optional.push(is_optional);
    }
    let result = method.function.return_type.as_ref().ok_or_else(|| {
        format!("generic method `{class}.{method_name}` needs a return annotation")
    })?;
    let result = generic_type_pattern(
        &result.type_ann,
        &substitutions,
        interfaces,
        generic_interfaces,
        &mut Vec::new(),
    )?;
    let convert = |ty: &TsType| {
        generic_type_pattern(
            ty,
            &substitutions,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )
    };
    Ok(Some(GenericClassMethodShape {
        parameters,
        optional,
        rest: method
            .function
            .params
            .last()
            .is_some_and(|parameter| matches!(parameter.pat, Pat::Rest(_))),
        result,
        constraints: type_parameters
            .params
            .iter()
            .map(|parameter| parameter.constraint.as_deref().map(&convert).transpose())
            .collect::<Result<_, _>>()?,
        defaults: type_parameters
            .params
            .iter()
            .map(|parameter| parameter.default.as_deref().map(&convert).transpose())
            .collect::<Result<_, _>>()?,
        is_async: method.function.is_async,
    }))
}

fn validate_abstract_generic_class_methods(
    module: &Module,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<(), String> {
    let classes = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) => {
                Some((class.ident.sym.to_string(), class))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    for (name, declaration) in &classes {
        if declaration.class.is_abstract {
            continue;
        }
        let mut selected = HashMap::<(bool, MethodKind, Symbol), (&str, &ClassMethod)>::new();
        let mut current = Some(name.as_str());
        while let Some(class_name) = current {
            let class = classes[class_name];
            for member in &class.class.body {
                let ClassMember::Method(method) = member else {
                    continue;
                };
                let method_name = class_property_name(&method.key)?;
                let key = (method.is_static, method.kind, method_name.clone());
                if let Some((implementation_class, implementation)) = selected.get(&key) {
                    if method.is_abstract && method.function.type_params.is_some() {
                        let required = generic_class_method_shape(
                            class_name,
                            &method_name,
                            method,
                            interfaces,
                            generic_interfaces,
                        )?;
                        let actual = generic_class_method_shape(
                            implementation_class,
                            &method_name,
                            implementation,
                            interfaces,
                            generic_interfaces,
                        )?;
                        if required != actual {
                            return Err(format!(
                                "class `{name}` implements abstract generic member `{method_name}` from `{class_name}` with an incompatible signature"
                            ));
                        }
                    }
                    continue;
                }
                selected.insert(key, (class_name, method));
                if method.is_abstract && method.function.type_params.is_some() {
                    return Err(format!(
                        "concrete class `{name}` must implement abstract generic member `{method_name}` from `{class_name}`"
                    ));
                }
            }
            current = class
                .class
                .super_class
                .as_deref()
                .and_then(|parent| parent.as_ident())
                .map(|parent| parent.sym.as_ref());
        }
    }
    Ok(())
}

fn specialize_generic_class_methods(
    module: &Module,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Option<Module>, String> {
    validate_abstract_generic_class_methods(module, interfaces, generic_interfaces)?;
    let mut templates = HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        for member in &declaration.class.body {
            let ClassMember::Method(method) = member else {
                continue;
            };
            let Some(parameters) = method.function.type_params.as_ref() else {
                continue;
            };
            if method.kind != MethodKind::Method {
                return Err(format!(
                    "generic class accessor `{}.{}` is not supported",
                    declaration.ident.sym,
                    class_property_name(&method.key)?
                ));
            }
            validate_trailing_type_parameter_defaults(
                "generic method",
                &format!(
                    "{}.{}",
                    declaration.ident.sym,
                    class_property_name(&method.key)?
                ),
                parameters,
            )?;
            let parameter_names = parameters
                .params
                .iter()
                .map(|parameter| parameter.name.sym.to_string())
                .collect::<Vec<_>>();
            let substitutions = parameter_names
                .iter()
                .map(|parameter| {
                    (
                        parameter.clone(),
                        GenericTypePattern::Variable(parameter.clone()),
                    )
                })
                .collect::<HashMap<_, _>>();
            let parameter_patterns = method
                .function
                .params
                .iter()
                .map(|parameter| {
                    let annotation = match &parameter.pat {
                        Pat::Ident(binding) => binding.type_ann.as_ref(),
                        Pat::Assign(assignment) => match assignment.left.as_ref() {
                            Pat::Ident(binding) => binding.type_ann.as_ref(),
                            _ => None,
                        },
                        Pat::Rest(rest) => rest.type_ann.as_ref(),
                        _ => None,
                    };
                    let annotation = annotation.ok_or_else(|| {
                        format!(
                            "generic method `{}.{}` inference requires annotated identifier parameters",
                            declaration.ident.sym,
                            class_property_name(&method.key).unwrap_or_default()
                        )
                    })?;
                    generic_type_pattern(
                        &annotation.type_ann,
                        &substitutions,
                        interfaces,
                        generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            let rest_pattern = method
                .function
                .params
                .last()
                .and_then(|parameter| matches!(parameter.pat, Pat::Rest(_)).then_some(()))
                .and_then(|()| parameter_patterns.last())
                .and_then(|pattern| match pattern {
                    GenericTypePattern::Array(element) => Some(element.as_ref().clone()),
                    _ => None,
                });
            templates.insert(
                (
                    declaration.ident.sym.to_string(),
                    class_property_name(&method.key)?,
                ),
                GenericClassMethodTemplate {
                    method: method.clone(),
                    parameters: parameter_names,
                    constraints: parameters
                        .params
                        .iter()
                        .map(|parameter| parameter.constraint.clone())
                        .collect(),
                    defaults: parameters
                        .params
                        .iter()
                        .map(|parameter| parameter.default.clone())
                        .collect(),
                    parameter_patterns,
                    rest_pattern,
                },
            );
        }
    }
    if templates.is_empty() {
        return Ok(None);
    }

    let call_results = generic_constructor_call_results(module, interfaces, generic_interfaces);
    let parents = module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
                return None;
            };
            let Expr::Ident(parent) = declaration.class.super_class.as_deref()? else {
                return None;
            };
            Some((declaration.ident.sym.to_string(), parent.sym.to_string()))
        })
        .collect::<HashMap<_, _>>();
    let classes = module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
                return None;
            };
            Some(declaration.ident.sym.to_string())
        })
        .collect::<HashSet<_>>();
    let mut static_member_types = HashMap::new();
    let mut instance_member_types = HashMap::new();
    let mut instance_method_results = HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        let class = declaration.ident.sym.to_string();
        for member in &declaration.class.body {
            let (name, ty) = match member {
                ClassMember::ClassProp(property) if property.is_static => {
                    let Some(annotation) = property.type_ann.as_ref() else {
                        continue;
                    };
                    let mut ty =
                        lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?;
                    if property.is_optional {
                        ty = optional_parameter_type(ty);
                    }
                    (class_property_name(&property.key)?, ty)
                }
                ClassMember::Method(method)
                    if method.is_static
                        && method.function.type_params.is_none()
                        && method.kind != MethodKind::Setter =>
                {
                    let Some(annotation) = method.function.return_type.as_ref() else {
                        continue;
                    };
                    let mut ty =
                        lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?;
                    if method.function.is_async && !matches!(ty, HirType::Promise(_)) {
                        ty = HirType::Promise(Box::new(ty));
                    }
                    (class_property_name(&method.key)?, ty)
                }
                _ => continue,
            };
            static_member_types.insert((class.clone(), name), ty);
        }
        for member in &declaration.class.body {
            let (name, mut ty, method_result) = match member {
                ClassMember::ClassProp(property) if !property.is_static => {
                    let Some(annotation) = property.type_ann.as_ref() else {
                        continue;
                    };
                    let Ok(mut ty) =
                        lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                    else {
                        continue;
                    };
                    if property.is_optional {
                        ty = optional_parameter_type(ty);
                    }
                    (class_property_name(&property.key)?, ty, false)
                }
                ClassMember::Method(method)
                    if !method.is_static
                        && method.function.type_params.is_none()
                        && method.kind != MethodKind::Setter =>
                {
                    let Some(annotation) = method.function.return_type.as_ref() else {
                        continue;
                    };
                    let Ok(ty) =
                        lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                    else {
                        continue;
                    };
                    (
                        class_property_name(&method.key)?,
                        ty,
                        method.kind == MethodKind::Method,
                    )
                }
                _ => continue,
            };
            if method_result && matches!(ty, HirType::Void) {
                continue;
            }
            if let ClassMember::Method(method) = member {
                if method.function.is_async && !matches!(ty, HirType::Promise(_)) {
                    ty = HirType::Promise(Box::new(ty));
                }
            }
            let types = if method_result {
                &mut instance_method_results
            } else {
                &mut instance_member_types
            };
            types.insert((class.clone(), name), ty);
        }
    }
    let mut collector = GenericClassMethodUseCollector {
        templates: &templates,
        interfaces,
        generic_interfaces,
        scopes: vec![HashMap::new()],
        uses: Vec::new(),
        call_results: &call_results,
        parents: &parents,
        static_member_types: &static_member_types,
        instance_member_types: &instance_member_types,
        instance_method_results: &instance_method_results,
        classes: &classes,
        current_classes: Vec::new(),
        current_static_contexts: Vec::new(),
        error: None,
    };
    module.visit_with(&mut collector);
    if let Some(error) = collector.error {
        return Err(error);
    }
    let has_uses = !collector.uses.is_empty();
    let mut instances = Vec::<(Symbol, Symbol, Vec<HirType>, Symbol)>::new();
    let mut calls = HashMap::new();
    let mut generated = HashMap::<Symbol, Vec<ClassMethod>>::new();
    for usage in collector.uses {
        let template = &templates[&(usage.class.clone(), usage.method.clone())];
        let (types, arguments) = resolve_explicit_generic_class_method_types(
            &usage.class,
            &usage.method,
            template,
            usage.arguments.as_deref(),
            usage.actual_params.as_deref(),
            interfaces,
            generic_interfaces,
        )?;
        if let Some(unsupported) = types.iter().find(|ty| !supports_generic_native_layout(ty)) {
            return Err(format!(
                "generic method `{}.{}` cannot specialize for native layout {unsupported:?}",
                usage.class, usage.method
            ));
        }
        let specialized_name = if let Some(existing) =
            instances
                .iter()
                .find_map(|(class, method, candidate_types, symbol)| {
                    (class == &usage.class && method == &usage.method && candidate_types == &types)
                        .then(|| symbol.clone())
                }) {
            existing
        } else {
            let specialized_name = specialized_generic_name(&usage.method, &types);
            let already_generated = module.body.iter().any(|item| {
                matches!(item, ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration)))
                if declaration.ident.sym.as_ref() == usage.class
                    && declaration.class.body.iter().any(|member| {
                        matches!(member, ClassMember::Method(method)
                            if method.function.type_params.is_none()
                                && class_property_name(&method.key).ok().as_deref()
                                    == Some(specialized_name.as_str()))
                    }))
            });
            if !already_generated {
                let substitutions = template
                    .parameters
                    .iter()
                    .cloned()
                    .zip(arguments.into_iter().map(Box::new))
                    .collect::<HashMap<_, _>>();
                let mut method = template.method.clone();
                method.key =
                    PropName::Ident(IdentName::new(specialized_name.clone().into(), method.span));
                method.function.type_params = None;
                method
                    .function
                    .visit_mut_with(&mut GenericClassTypeSubstituter {
                        substitutions: &substitutions,
                    });
                generated
                    .entry(usage.class.clone())
                    .or_default()
                    .push(method);
            }
            instances.push((
                usage.class.clone(),
                usage.method.clone(),
                types,
                specialized_name.clone(),
            ));
            specialized_name
        };
        calls.insert(usage.span, specialized_name);
    }

    let mut specialized = module.clone();
    for item in &mut specialized.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        let class = declaration.ident.sym.to_string();
        declaration.class.body.retain(|member| {
            !matches!(member, ClassMember::Method(method)
                if method.function.type_params.is_some()
                    && !has_uses)
        });
        if let Some(methods) = generated.remove(&class) {
            declaration
                .class
                .body
                .extend(methods.into_iter().map(ClassMember::Method));
        }
    }
    specialized.visit_mut_with(&mut GenericClassMethodCallRewriter { calls: &calls });
    Ok(Some(specialized))
}
