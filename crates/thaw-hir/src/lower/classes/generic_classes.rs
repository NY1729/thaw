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
        HirType::JsValue => TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
            span: swc_common::DUMMY_SP,
            type_name: swc_ecma_ast::TsEntityName::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                "JsValue".into(),
                swc_common::DUMMY_SP,
            )),
            type_params: None,
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
    fn visit_class_decl(&mut self, declaration: &swc_ecma_ast::ClassDecl) {
        if declaration.class.type_params.is_none() {
            declaration.visit_children_with(self);
        }
    }

    fn visit_class_method(&mut self, method: &swc_ecma_ast::ClassMethod) {
        if method.function.type_params.is_none() {
            method.visit_children_with(self);
        }
    }

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
        fn visit_class_method(&mut self, method: &swc_ecma_ast::ClassMethod) {
            if method.function.type_params.is_none() {
                method.visit_children_with(self);
            }
        }

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
    fn visit_mut_class_method(&mut self, method: &mut swc_ecma_ast::ClassMethod) {
        if method.function.type_params.is_none() {
            method.visit_mut_children_with(self);
        }
    }

    fn visit_mut_class(&mut self, class: &mut swc_ecma_ast::Class) {
        if class.type_params.is_some() {
            return;
        }
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
    let mut existing_static_owners = HashSet::new();
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
            existing_static_owners.insert(owner.clone());
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
        if existing_static_owners.contains(owner) {
            continue;
        }
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
    for (name, template) in &templates {
        if static_owners.contains_key(name)
            && !existing_static_owners.contains(&generic_class_static_owner(name))
        {
            specialized
                .body
                .push(ModuleItem::Stmt(Stmt::Decl(Decl::Class(
                    template.declaration.clone(),
                ))));
        }
    }
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
                    if declaration.class.type_params.is_some()
                        || declaration.class.super_type_params.is_some()
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
    if instances.is_empty() {
        let has_pending_generic_methods = module.body.iter().any(|item| {
            matches!(item, ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration)))
                if declaration.class.type_params.is_none()
                    && declaration.class.body.iter().any(|member| {
                        matches!(member, ClassMember::Method(method)
                            if method.function.type_params.is_some())
                    }))
        });
        if has_pending_generic_methods {
            return Ok(None);
        }
        let mut cleaned = specialized;
        cleaned.body.retain(|item| {
            !matches!(item, ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration)))
                if declaration.class.type_params.is_some())
        });
        return Ok(Some(cleaned));
    }
    if specialized.body.iter().any(|item| {
        matches!(
            item,
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration)))
                if declaration.class.type_params.is_none()
                    && class_contains_unresolved_generic_type(&declaration.class, &names)
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
