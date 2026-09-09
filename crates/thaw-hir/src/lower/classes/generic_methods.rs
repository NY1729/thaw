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

    fn contextual_argument_type(
        &self,
        expression: &Expr,
        pattern: &GenericTypePattern,
        inferred: &HashMap<Symbol, HirType>,
    ) -> Result<HirType, String> {
        let (Expr::Arrow(arrow), GenericTypePattern::Function(params, _, _, _)) =
            (expression, pattern)
        else {
            return self.infer_actual_type(expression);
        };
        if arrow.params.len() != params.len() {
            return self.infer_actual_type(expression);
        }
        let params = params
            .iter()
            .map(|param| instantiate_generic_pattern(param, inferred))
            .collect::<Result<Vec<_>, _>>()?;
        let mut scopes = self.scopes.clone();
        let mut scope = HashMap::new();
        for (parameter, ty) in arrow.params.iter().zip(&params) {
            let Pat::Ident(binding) = parameter else {
                return Err("contextual generic callback requires identifier parameters".into());
            };
            scope.insert(binding.id.sym.to_string(), ty.clone());
        }
        scopes.push(scope);
        let result = if let Some(annotation) = arrow.return_type.as_ref() {
            lower_ts_type(&annotation.type_ann, self.interfaces, self.generic_interfaces)?
        } else {
            let ArrowFunctionBody::Expr(body) = arrow.body.as_ref() else {
                return Err(
                    "block-bodied contextual generic callback needs a return annotation".into(),
                );
            };
            infer_generic_constructor_expr_type(
                body,
                self.interfaces,
                self.generic_interfaces,
                &scopes,
                self.call_results,
            )?
        };
        Ok(HirType::Function(params, Box::new(result)))
    }

    fn call_actual_params(
        &self,
        call: &CallExpr,
        template: &GenericClassMethodTemplate,
    ) -> Result<Vec<HirType>, String> {
        let mut actual = Vec::new();
        let mut inferred = HashMap::new();
        let fixed_count = template
            .parameter_patterns
            .len()
            .saturating_sub(usize::from(template.rest_pattern.is_some()));
        for (index, argument) in call.args.iter().enumerate() {
            let pattern = (index < fixed_count)
                .then(|| &template.parameter_patterns[index])
                .or(template.rest_pattern.as_ref())
                .ok_or("generic method received too many arguments")?;
            let ty = self.contextual_argument_type(&argument.expr, pattern, &inferred)?;
            if argument.spread.is_none() {
                match_generic_pattern(pattern, &ty, &mut inferred)?;
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
            let ty = declarator
                .init
                .as_deref()
                .and_then(|initializer| match initializer {
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
                })
                .or_else(|| {
                    binding.type_ann.as_ref().and_then(|annotation| {
                        lower_ts_type(
                            &annotation.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                        .ok()
                    })
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
                            match self.call_actual_params(
                                call,
                                &self.templates[&(owner.clone(), method.clone())],
                            ) {
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
                            match self.call_actual_params(
                                call,
                                &self.templates[&(owner.clone(), method.clone())],
                            ) {
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
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // `lower/module/classes.rs`'s synthetic base-layout branch) and
            // so have no abstract generic methods of their own to walk.
            let Some(&class) = classes.get(class_name) else {
                break;
            };
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
        if declaration.class.type_params.is_some() {
            continue;
        }
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
        if !has_uses && templates.keys().any(|(owner, _)| owner == &class) {
            // `implements` was checked before specialization. Once its generic
            // method template is removed, checking the rewritten class again
            // would mistake the generated, type-specific methods for a missing
            // source method.
            declaration.class.implements.clear();
        }
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
