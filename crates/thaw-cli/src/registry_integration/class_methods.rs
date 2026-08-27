#[cfg(test)]
fn rewrite_external_class_methods(
    source: &str,
    classes: &[ClassConstructorRewrite],
    methods: &[ClassMethodRewrite],
) -> Result<String, String> {
    rewrite_external_class_methods_with_static(source, classes, methods, &[], &[], &[], &[], &[])
}

#[allow(clippy::too_many_arguments)]
fn rewrite_external_class_methods_with_static(
    source: &str,
    classes: &[ClassConstructorRewrite],
    methods: &[ClassMethodRewrite],
    static_methods: &[StaticClassMethodRewrite],
    getters: &[ClassGetterRewrite],
    setters: &[ClassSetterRewrite],
    static_getters: &[StaticClassGetterRewrite],
    static_setters: &[StaticClassSetterRewrite],
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowFunctionBody, AssignExpr, AssignOp, AssignTarget, BinaryOp, BreakStmt, CallExpr,
        Callee, DoWhileStmt, Expr, FnDecl, ForInStmt, ForOfStmt, ForStmt, FunctionBody, IfStmt,
        Lit, MemberProp, Pat, Prop, PropName, PropOrSpread, ReturnStmt, SimpleAssignTarget, Stmt,
        SwitchStmt, TryStmt, TsKeywordTypeKind, TsType, UnaryOp, VarDeclarator, WhileStmt,
    };
    use thaw_parser::common::Spanned;

    if methods.is_empty()
        && static_methods.is_empty()
        && getters.is_empty()
        && setters.is_empty()
        && static_getters.is_empty()
        && static_setters.is_empty()
    {
        return Ok(source.to_string());
    }

    fn constructed_class<'a>(
        expression: &'a Expr,
        classes: &'a [ClassConstructorRewrite],
    ) -> Option<&'a str> {
        let Expr::New(new_expression) = expression else {
            return None;
        };
        match new_expression.callee.as_ref() {
            Expr::Ident(class) => classes
                .iter()
                .find(|(_, name)| name == class.sym.as_str())
                .map(|(_, name)| name.as_str()),
            Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                (Expr::Ident(package), MemberProp::Ident(class)) => classes
                    .iter()
                    .find(|(qualifier, name)| {
                        qualifier == package.sym.as_str() && name == class.sym.as_str()
                    })
                    .map(|(_, name)| name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    fn source_instance_class(
        expression: &Expr,
        classes: &[ClassConstructorRewrite],
        variables: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        match expression {
            Expr::Ident(identifier) => variables.get(identifier.sym.as_str()).cloned(),
            Expr::Member(member) => member_assignment_path(member)
                .map(|(root, path)| instance_path_key(&root, &path))
                .and_then(|key| variables.get(&key).cloned()),
            Expr::Paren(parenthesized) => {
                source_instance_class(&parenthesized.expr, classes, variables)
            }
            Expr::TsAs(assertion) => source_instance_class(&assertion.expr, classes, variables),
            Expr::TsTypeAssertion(assertion) => {
                source_instance_class(&assertion.expr, classes, variables)
            }
            _ => constructed_class(expression, classes).map(str::to_owned),
        }
    }

    fn instance_path_key(root: &str, path: &[String]) -> String {
        let mut key = root.to_string();
        for property in path {
            key.push('\u{1f}');
            key.push_str(property);
        }
        key
    }

    fn invalidate_instance_path(
        variables: &mut std::collections::HashMap<String, String>,
        key: &str,
    ) {
        let descendant = format!("{key}\u{1f}");
        variables.retain(|candidate, _| candidate != key && !candidate.starts_with(&descendant));
    }

    fn instance_receiver(expression: &Expr) -> Option<(String, String)> {
        match expression {
            Expr::Ident(identifier) => {
                let name = identifier.sym.to_string();
                Some((name.clone(), name))
            }
            Expr::Member(member) => {
                let (key, rendered) = instance_receiver(&member.obj)?;
                match &member.prop {
                    MemberProp::Ident(property) => Some((
                        format!("{key}\u{1f}{}", property.sym),
                        format!("{rendered}.{}", property.sym),
                    )),
                    MemberProp::Computed(computed) => {
                        let Expr::Lit(Lit::Str(property)) = computed.expr.as_ref() else {
                            return None;
                        };
                        let property = property.value.to_string_lossy().into_owned();
                        Some((
                            format!("{key}\u{1f}{property}"),
                            format!("{rendered}[{}]", serde_json::to_string(&property).ok()?),
                        ))
                    }
                    MemberProp::PrivateName(_) => None,
                }
            }
            Expr::Paren(parenthesized) => instance_receiver(&parenthesized.expr),
            Expr::TsAs(assertion) => instance_receiver(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => instance_receiver(&assertion.expr),
            _ => None,
        }
    }

    fn static_class_receiver(expression: &Expr) -> Option<(Option<String>, String)> {
        match expression {
            Expr::Ident(class) => Some((None, class.sym.to_string())),
            Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                (Expr::Ident(qualifier), MemberProp::Ident(class)) => {
                    Some((Some(qualifier.sym.to_string()), class.sym.to_string()))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn collect_object_instance_classes(
        expression: &Expr,
        prefix: &str,
        classes: &[ClassConstructorRewrite],
        variables: &std::collections::HashMap<String, String>,
        additions: &mut Vec<(String, String)>,
    ) {
        let Expr::Object(object) = expression else {
            return;
        };
        for property in &object.props {
            let PropOrSpread::Prop(property) = property else {
                continue;
            };
            let (name, value): (String, &Expr) = match property.as_ref() {
                Prop::KeyValue(property) => {
                    let Some(name) = (match &property.key {
                        PropName::Ident(identifier) => Some(identifier.sym.to_string()),
                        PropName::Str(value) => Some(value.value.to_string_lossy().into_owned()),
                        PropName::Computed(computed) => match computed.expr.as_ref() {
                            Expr::Lit(Lit::Str(value)) => {
                                Some(value.value.to_string_lossy().into_owned())
                            }
                            _ => None,
                        },
                        _ => None,
                    }) else {
                        continue;
                    };
                    (name, property.value.as_ref())
                }
                Prop::Shorthand(identifier) => {
                    let key = format!("{prefix}\u{1f}{}", identifier.sym);
                    if let Some(class) = variables.get(identifier.sym.as_str()) {
                        additions.push((key, class.clone()));
                    }
                    continue;
                }
                _ => continue,
            };
            let key = format!("{prefix}\u{1f}{name}");
            if let Some(class) = source_instance_class(value, classes, variables) {
                additions.push((key.clone(), class));
            }
            collect_object_instance_classes(value, &key, classes, variables, additions);
        }
    }

    fn source_expr_type(
        expression: &Expr,
        variables: &std::collections::HashMap<String, thaw_hir::HirType>,
        functions: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        match expression {
            Expr::Lit(Lit::Num(_)) => Some(thaw_hir::HirType::F64),
            Expr::Lit(Lit::Str(_)) => Some(thaw_hir::HirType::Str),
            Expr::Lit(Lit::Bool(_)) => Some(thaw_hir::HirType::Bool),
            Expr::Array(array)
                if array.elems.iter().all(|element| {
                    element.as_ref().is_some_and(|element| {
                        source_expr_type(element.expr.as_ref(), variables, functions)
                            == Some(thaw_hir::HirType::F64)
                    })
                }) =>
            {
                Some(thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)))
            }
            Expr::Object(object) => {
                let mut fields = Vec::with_capacity(object.props.len());
                for property in &object.props {
                    if let PropOrSpread::Spread(spread) = property {
                        let Some(thaw_hir::HirType::Object(spread_fields)) =
                            source_expr_type(&spread.expr, variables, functions)
                        else {
                            return Some(thaw_hir::HirType::Object(Vec::new()));
                        };
                        for (name, value_type) in spread_fields {
                            fields.retain(|(existing, _)| existing != &name);
                            fields.push((name, value_type));
                        }
                        continue;
                    }
                    let PropOrSpread::Prop(property) = property else {
                        unreachable!();
                    };
                    let (name, value_type) = match property.as_ref() {
                        Prop::KeyValue(property) => {
                            let name = match &property.key {
                                PropName::Ident(identifier) => identifier.sym.to_string(),
                                PropName::Str(value) => value.value.to_string_lossy().into_owned(),
                                PropName::Computed(computed) => match computed.expr.as_ref() {
                                    Expr::Lit(Lit::Str(value)) => {
                                        value.value.to_string_lossy().into_owned()
                                    }
                                    _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                                },
                                _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                            };
                            let Some(value_type) =
                                source_expr_type(&property.value, variables, functions)
                            else {
                                return Some(thaw_hir::HirType::Object(Vec::new()));
                            };
                            (name, value_type)
                        }
                        Prop::Shorthand(identifier) => {
                            let Some(value_type) = variables.get(identifier.sym.as_str()).cloned()
                            else {
                                return Some(thaw_hir::HirType::Object(Vec::new()));
                            };
                            (identifier.sym.to_string(), value_type)
                        }
                        _ => return Some(thaw_hir::HirType::Object(Vec::new())),
                    };
                    fields.retain(|(existing, _)| existing != &name);
                    fields.push((name, value_type));
                }
                Some(thaw_hir::HirType::Object(fields))
            }
            Expr::Ident(identifier) => variables.get(identifier.sym.as_str()).cloned(),
            Expr::Paren(parenthesized) => {
                source_expr_type(&parenthesized.expr, variables, functions)
            }
            Expr::TsAs(assertion) => source_expr_type(&assertion.expr, variables, functions),
            Expr::TsTypeAssertion(assertion) => {
                source_expr_type(&assertion.expr, variables, functions)
            }
            Expr::Tpl(_) => Some(thaw_hir::HirType::Str),
            Expr::Unary(unary) => match unary.op {
                UnaryOp::Plus | UnaryOp::Minus
                    if source_expr_type(&unary.arg, variables, functions)
                        == Some(thaw_hir::HirType::F64) =>
                {
                    Some(thaw_hir::HirType::F64)
                }
                UnaryOp::Bang => Some(thaw_hir::HirType::Bool),
                _ => None,
            },
            Expr::Bin(binary) => {
                let left = source_expr_type(&binary.left, variables, functions);
                let right = source_expr_type(&binary.right, variables, functions);
                match binary.op {
                    BinaryOp::Add
                        if left == Some(thaw_hir::HirType::Str)
                            && right == Some(thaw_hir::HirType::Str) =>
                    {
                        Some(thaw_hir::HirType::Str)
                    }
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Exp
                        if left == Some(thaw_hir::HirType::F64)
                            && right == Some(thaw_hir::HirType::F64) =>
                    {
                        Some(thaw_hir::HirType::F64)
                    }
                    BinaryOp::EqEq
                    | BinaryOp::NotEq
                    | BinaryOp::EqEqEq
                    | BinaryOp::NotEqEq
                    | BinaryOp::Lt
                    | BinaryOp::LtEq
                    | BinaryOp::Gt
                    | BinaryOp::GtEq => Some(thaw_hir::HirType::Bool),
                    _ => None,
                }
            }
            Expr::Cond(conditional) => {
                let consequent = source_expr_type(&conditional.cons, variables, functions);
                let alternate = source_expr_type(&conditional.alt, variables, functions);
                (consequent == alternate).then_some(consequent).flatten()
            }
            Expr::Call(call) => match &call.callee {
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(identifier) if identifier.sym == *"Number" => {
                        Some(thaw_hir::HirType::F64)
                    }
                    Expr::Ident(identifier) if identifier.sym == *"String" => {
                        Some(thaw_hir::HirType::Str)
                    }
                    Expr::Ident(identifier) if identifier.sym == *"Boolean" => {
                        Some(thaw_hir::HirType::Bool)
                    }
                    Expr::Ident(identifier) => functions.get(identifier.sym.as_str()).cloned(),
                    _ => None,
                },
                _ => None,
            },
            Expr::Member(member) => {
                let MemberProp::Ident(property) = &member.prop else {
                    return None;
                };
                if property.sym == *"length" {
                    return Some(thaw_hir::HirType::F64);
                }
                let thaw_hir::HirType::Object(fields) =
                    source_expr_type(&member.obj, variables, functions)?
                else {
                    return None;
                };
                fields
                    .into_iter()
                    .find(|(name, _)| name == property.sym.as_str())
                    .map(|(_, ty)| ty)
            }
            _ => None,
        }
    }

    fn source_ts_type(ty: &TsType) -> Option<thaw_hir::HirType> {
        match ty {
            TsType::TsKeywordType(keyword) => match keyword.kind {
                TsKeywordTypeKind::TsNumberKeyword => Some(thaw_hir::HirType::F64),
                TsKeywordTypeKind::TsStringKeyword => Some(thaw_hir::HirType::Str),
                TsKeywordTypeKind::TsBooleanKeyword => Some(thaw_hir::HirType::Bool),
                TsKeywordTypeKind::TsVoidKeyword => Some(thaw_hir::HirType::Void),
                _ => None,
            },
            TsType::TsArrayType(array) => match source_ts_type(&array.elem_type) {
                Some(thaw_hir::HirType::F64) => {
                    Some(thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)))
                }
                _ => None,
            },
            TsType::TsParenthesizedType(parenthesized) => source_ts_type(&parenthesized.type_ann),
            _ => None,
        }
    }

    #[derive(Default)]
    struct FunctionTypeFinder {
        types: std::collections::HashMap<String, thaw_hir::HirType>,
    }

    impl Visit for FunctionTypeFinder {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            if let Some(return_type) = declaration
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| source_ts_type(&annotation.type_ann))
            {
                self.types
                    .insert(declaration.ident.sym.to_string(), return_type);
            }
            declaration.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                let annotation = match initializer.as_ref() {
                    Expr::Arrow(arrow) => arrow.return_type.as_ref(),
                    Expr::Fn(function) => function.function.return_type.as_ref(),
                    _ => None,
                };
                if let Some(return_type) =
                    annotation.and_then(|annotation| source_ts_type(&annotation.type_ann))
                {
                    self.types.insert(binding.id.sym.to_string(), return_type);
                }
            }
            declaration.visit_children_with(self);
        }
    }

    fn inferred_block_return_type(
        block: &FunctionBody,
        functions: &std::collections::HashMap<String, thaw_hir::HirType>,
    ) -> Option<thaw_hir::HirType> {
        struct Returns<'a> {
            functions: &'a std::collections::HashMap<String, thaw_hir::HirType>,
            types: Vec<Option<thaw_hir::HirType>>,
        }
        impl Visit for Returns<'_> {
            fn visit_return_stmt(&mut self, statement: &ReturnStmt) {
                self.types
                    .push(statement.arg.as_ref().and_then(|expression| {
                        source_expr_type(
                            expression,
                            &std::collections::HashMap::new(),
                            self.functions,
                        )
                    }));
            }

            // Returns inside nested functions belong to those functions, not
            // to the block currently being inferred.
            fn visit_fn_decl(&mut self, _declaration: &FnDecl) {}

            fn visit_arrow_expr(&mut self, _expression: &thaw_parser::ast::ArrowExpr) {}

            fn visit_fn_expr(&mut self, _expression: &thaw_parser::ast::FnExpr) {}
        }

        let mut returns = Returns {
            functions,
            types: Vec::new(),
        };
        block.visit_with(&mut returns);
        let first = returns.types.first()?.clone()?;
        returns
            .types
            .iter()
            .all(|candidate| candidate.as_ref() == Some(&first))
            .then_some(first)
    }

    struct InferredFunctionTypeFinder<'a> {
        known: &'a std::collections::HashMap<String, thaw_hir::HirType>,
        additions: std::collections::HashMap<String, thaw_hir::HirType>,
    }

    impl Visit for InferredFunctionTypeFinder<'_> {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            if !self.known.contains_key(declaration.ident.sym.as_str()) {
                if let Some(return_type) = declaration
                    .function
                    .body
                    .as_ref()
                    .and_then(|body| inferred_block_return_type(body, self.known))
                {
                    self.additions
                        .insert(declaration.ident.sym.to_string(), return_type);
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                if !self.known.contains_key(binding.id.sym.as_str()) {
                    let return_type = match initializer.as_ref() {
                        Expr::Arrow(arrow) => match arrow.body.as_ref() {
                            ArrowFunctionBody::FunctionBody(block) => {
                                inferred_block_return_type(block, self.known)
                            }
                            ArrowFunctionBody::Expr(expression) => source_expr_type(
                                expression,
                                &std::collections::HashMap::new(),
                                self.known,
                            ),
                        },
                        Expr::Fn(function) => function
                            .function
                            .body
                            .as_ref()
                            .and_then(|body| inferred_block_return_type(body, self.known)),
                        _ => None,
                    };
                    if let Some(return_type) = return_type {
                        self.additions
                            .insert(binding.id.sym.to_string(), return_type);
                    }
                }
            }
            declaration.visit_children_with(self);
        }
    }

    fn overload_type_score(declared: &thaw_hir::HirType, actual: &thaw_hir::HirType) -> Option<u8> {
        match (declared, actual) {
            (thaw_hir::HirType::Json, _) => Some(0),
            (thaw_hir::HirType::Object(declared), thaw_hir::HirType::Object(actual)) => {
                if declared.is_empty() || actual.is_empty() {
                    return Some(1);
                }
                let mut score = 2u8;
                for (name, declared_type) in declared {
                    let (_, actual_type) =
                        actual.iter().find(|(candidate, _)| candidate == name)?;
                    score = score.saturating_add(overload_type_score(declared_type, actual_type)?);
                }
                Some(score)
            }
            (left, right) if left == right => Some(2),
            _ => None,
        }
    }

    fn member_property_name(property: &MemberProp) -> Option<String> {
        match property {
            MemberProp::Ident(identifier) => Some(identifier.sym.to_string()),
            MemberProp::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
                _ => None,
            },
            MemberProp::PrivateName(_) => None,
        }
    }

    fn member_assignment_path(
        member: &thaw_parser::ast::MemberExpr,
    ) -> Option<(String, Vec<String>)> {
        let mut path = vec![member_property_name(&member.prop)?];
        let mut object = member.obj.as_ref();
        loop {
            match object {
                Expr::Ident(identifier) => {
                    path.reverse();
                    return Some((identifier.sym.to_string(), path));
                }
                Expr::Member(parent) => {
                    path.push(member_property_name(&parent.prop)?);
                    object = parent.obj.as_ref();
                }
                _ => return None,
            }
        }
    }

    fn update_object_property_type(
        object: &mut thaw_hir::HirType,
        path: &[String],
        value: thaw_hir::HirType,
    ) -> bool {
        let thaw_hir::HirType::Object(fields) = object else {
            return false;
        };
        let Some((property, remaining)) = path.split_first() else {
            return false;
        };
        if remaining.is_empty() {
            if let Some((_, ty)) = fields.iter_mut().find(|(name, _)| name == property) {
                *ty = value;
            } else {
                fields.push((property.clone(), value));
            }
            return true;
        }
        fields
            .iter_mut()
            .find(|(name, _)| name == property)
            .is_some_and(|(_, nested)| update_object_property_type(nested, remaining, value))
    }

    struct Finder<'a> {
        classes: &'a [ClassConstructorRewrite],
        methods: &'a [ClassMethodRewrite],
        static_methods: &'a [StaticClassMethodRewrite],
        getters: &'a [ClassGetterRewrite],
        setters: &'a [ClassSetterRewrite],
        static_getters: &'a [StaticClassGetterRewrite],
        static_setters: &'a [StaticClassSetterRewrite],
        variables: std::collections::HashMap<String, String>,
        value_types: std::collections::HashMap<String, thaw_hir::HirType>,
        function_types: &'a std::collections::HashMap<String, thaw_hir::HirType>,
        callbacks: std::collections::HashSet<String>,
        edits: Vec<(u32, u32, String)>,
        switch_break_depth: usize,
        switch_break_exits: Vec<FlowState>,
    }

    #[derive(Clone)]
    struct FlowState {
        variables: std::collections::HashMap<String, String>,
        value_types: std::collections::HashMap<String, thaw_hir::HirType>,
        callbacks: std::collections::HashSet<String>,
    }

    impl Finder<'_> {
        fn flow_state(&self) -> FlowState {
            FlowState {
                variables: self.variables.clone(),
                value_types: self.value_types.clone(),
                callbacks: self.callbacks.clone(),
            }
        }

        fn restore_flow_state(&mut self, state: FlowState) {
            self.variables = state.variables;
            self.value_types = state.value_types;
            self.callbacks = state.callbacks;
        }

        fn join_current_flow_with(&mut self, other: &FlowState) {
            self.variables
                .retain(|name, class| other.variables.get(name) == Some(class));
            self.value_types
                .retain(|name, ty| other.value_types.get(name) == Some(ty));
            self.callbacks.retain(|name| other.callbacks.contains(name));
        }
    }

    impl Visit for Finder<'_> {
        fn visit_member_expr(&mut self, member: &thaw_parser::ast::MemberExpr) {
            if let (Some((qualifier, class)), MemberProp::Ident(property)) =
                (static_class_receiver(&member.obj), &member.prop)
            {
                if let Some((_, _, _, helper)) = self.static_getters.iter().find(
                    |(candidate_qualifier, candidate_class, candidate_property, _)| {
                        candidate_class == &class
                            && candidate_property == property.sym.as_str()
                            && qualifier
                                .as_ref()
                                .is_none_or(|qualifier| candidate_qualifier == qualifier)
                    },
                ) {
                    let span = member.span();
                    self.edits
                        .push((span.lo.0, span.hi.0, format!("{helper}()")));
                    return;
                }
            }
            if let (Some((receiver_key, receiver_source)), MemberProp::Ident(property)) =
                (instance_receiver(&member.obj), &member.prop)
            {
                if let Some(class) = self.variables.get(&receiver_key) {
                    if let Some((_, _, helper)) =
                        self.getters
                            .iter()
                            .find(|(candidate_class, candidate_property, _)| {
                                candidate_class == class
                                    && candidate_property == property.sym.as_str()
                            })
                    {
                        let span = member.span();
                        self.edits.push((
                            span.lo.0,
                            span.hi.0,
                            format!("{helper}({receiver_source})"),
                        ));
                        member.obj.visit_with(self);
                        return;
                    }
                }
            }
            member.visit_children_with(self);
        }

        fn visit_if_stmt(&mut self, statement: &IfStmt) {
            statement.test.visit_with(self);
            let base = self.flow_state();

            statement.cons.visit_with(self);
            let consequent = self.flow_state();

            self.restore_flow_state(base);
            if let Some(alternate) = &statement.alt {
                alternate.visit_with(self);
            }

            let alternate = self.flow_state();
            self.restore_flow_state(consequent);
            self.join_current_flow_with(&alternate);
        }

        fn visit_while_stmt(&mut self, statement: &WhileStmt) {
            statement.test.visit_with(self);
            let zero_iterations = self.flow_state();
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
            self.join_current_flow_with(&zero_iterations);
        }

        fn visit_for_stmt(&mut self, statement: &ForStmt) {
            if let Some(initializer) = &statement.init {
                initializer.visit_with(self);
            }
            if let Some(test) = &statement.test {
                test.visit_with(self);
            }
            let zero_iterations = self.flow_state();
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
            if let Some(update) = &statement.update {
                update.visit_with(self);
            }
            self.join_current_flow_with(&zero_iterations);
        }

        fn visit_do_while_stmt(&mut self, statement: &DoWhileStmt) {
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
            statement.test.visit_with(self);
        }

        fn visit_for_in_stmt(&mut self, statement: &ForInStmt) {
            statement.right.visit_with(self);
            statement.left.visit_with(self);
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
        }

        fn visit_for_of_stmt(&mut self, statement: &ForOfStmt) {
            statement.right.visit_with(self);
            statement.left.visit_with(self);
            self.switch_break_depth += usize::from(self.switch_break_depth > 0);
            statement.body.visit_with(self);
            self.switch_break_depth = self.switch_break_depth.saturating_sub(1);
        }

        fn visit_break_stmt(&mut self, statement: &BreakStmt) {
            if statement.label.is_none() && self.switch_break_depth == 1 {
                self.switch_break_exits.push(self.flow_state());
            }
        }

        fn visit_switch_stmt(&mut self, statement: &SwitchStmt) {
            let outer_break_depth = std::mem::replace(&mut self.switch_break_depth, 1);
            let outer_break_exits = std::mem::take(&mut self.switch_break_exits);
            statement.discriminant.visit_with(self);
            let base = self.flow_state();
            let mut direct_entries = vec![None; statement.cases.len()];

            // Case tests execute in source order until one matches. Visiting
            // each test once gives the exact incoming state for that direct
            // entry without duplicating source rewrites across possible paths.
            for (index, case) in statement.cases.iter().enumerate() {
                if let Some(test) = &case.test {
                    test.visit_with(self);
                    direct_entries[index] = Some(self.flow_state());
                }
            }
            let after_tests = self.flow_state();
            if let Some(default) = statement.cases.iter().position(|case| case.test.is_none()) {
                direct_entries[default] = Some(after_tests.clone());
            }

            let mut exits = Vec::new();
            if statement.cases.iter().all(|case| case.test.is_some()) {
                exits.push(after_tests);
            }
            let mut fallthrough: Option<FlowState> = None;

            for (case, direct) in statement.cases.iter().zip(direct_entries) {
                let mut incoming = direct.expect("every switch case has a direct entry");
                if let Some(previous) = fallthrough.take() {
                    self.restore_flow_state(incoming);
                    self.join_current_flow_with(&previous);
                    incoming = self.flow_state();
                }
                self.restore_flow_state(incoming);

                let mut terminated = false;
                for consequent in &case.cons {
                    consequent.visit_with(self);
                    if matches!(consequent, Stmt::Break(statement) if statement.label.is_none()) {
                        terminated = true;
                        break;
                    }
                }
                if !terminated {
                    fallthrough = Some(self.flow_state());
                }
            }
            if let Some(fallthrough) = fallthrough {
                exits.push(fallthrough);
            }
            exits.append(&mut self.switch_break_exits);
            self.switch_break_exits = outer_break_exits;
            self.switch_break_depth = outer_break_depth;

            let mut exits = exits.into_iter();
            let Some(first) = exits.next() else {
                self.restore_flow_state(base);
                return;
            };
            self.restore_flow_state(first);
            for exit in exits {
                self.join_current_flow_with(&exit);
            }
        }

        fn visit_try_stmt(&mut self, statement: &TryStmt) {
            let incoming = self.flow_state();
            statement.block.visit_with(self);
            let try_exit = self.flow_state();

            if let Some(handler) = &statement.handler {
                // A throw may occur after any prefix of the try block. Facts
                // that differ between entry and normal exit are therefore not
                // safe assumptions at catch entry.
                let mut catch_entry = try_exit.clone();
                catch_entry
                    .variables
                    .retain(|name, class| incoming.variables.get(name) == Some(class));
                catch_entry
                    .value_types
                    .retain(|name, ty| incoming.value_types.get(name) == Some(ty));
                catch_entry
                    .callbacks
                    .retain(|name| incoming.callbacks.contains(name));
                self.restore_flow_state(catch_entry);
                handler.body.visit_with(self);
                let catch_exit = self.flow_state();

                self.restore_flow_state(try_exit);
                self.join_current_flow_with(&catch_exit);
            }

            // `finally` executes on every path that leaves the construct, so
            // assignments made there can establish new facts after the join.
            if let Some(finalizer) = &statement.finalizer {
                finalizer.visit_with(self);
            }
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            if let (Pat::Ident(binding), Some(initializer)) = (&declaration.name, &declaration.init)
            {
                invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                if let Some(class) =
                    source_instance_class(initializer, self.classes, &self.variables)
                {
                    self.variables.insert(binding.id.sym.to_string(), class);
                }
                let mut property_classes = Vec::new();
                collect_object_instance_classes(
                    initializer,
                    binding.id.sym.as_str(),
                    self.classes,
                    &self.variables,
                    &mut property_classes,
                );
                self.variables.extend(property_classes);
                if matches!(initializer.as_ref(), Expr::Arrow(_) | Expr::Fn(_)) {
                    self.callbacks.insert(binding.id.sym.to_string());
                }
                if let Some(ty) =
                    source_expr_type(initializer, &self.value_types, self.function_types)
                {
                    self.value_types.insert(binding.id.sym.to_string(), ty);
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = callee.as_ref() {
                    let static_target = match member.obj.as_ref() {
                        Expr::Ident(class) => Some((None, class.sym.as_str())),
                        Expr::Member(class_member) => {
                            match (class_member.obj.as_ref(), &class_member.prop) {
                                (Expr::Ident(qualifier), MemberProp::Ident(class)) => {
                                    Some((Some(qualifier.sym.as_str()), class.sym.as_str()))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let (Some((qualifier, class)), MemberProp::Ident(method)) =
                        (static_target, &member.prop)
                    {
                        let has_callback = call.args.last().is_some_and(|argument| {
                            matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                                || matches!(argument.expr.as_ref(), Expr::Ident(identifier) if self.callbacks.contains(identifier.sym.as_str()))
                        });
                        let selected = self
                            .static_methods
                            .iter()
                            .filter(|candidate| {
                                candidate.1 == class
                                    && candidate.2 == method.sym.as_str()
                                    && candidate.4 == call.args.len()
                                    && candidate.5 == has_callback
                                    && qualifier.is_none_or(|qualifier| candidate.0 == qualifier)
                            })
                            .filter_map(|candidate| {
                                let mut score = 0u16;
                                for (argument, declared) in call.args.iter().zip(&candidate.6) {
                                    if let Some(actual) = source_expr_type(
                                        argument.expr.as_ref(),
                                        &self.value_types,
                                        self.function_types,
                                    ) {
                                        score += u16::from(overload_type_score(declared, &actual)?);
                                    }
                                }
                                Some((score, candidate))
                            })
                            .reduce(|best, candidate| {
                                if candidate.0 > best.0 {
                                    candidate
                                } else {
                                    best
                                }
                            })
                            .map(|(_, candidate)| candidate);
                        if let Some(candidate) = selected {
                            let span = member.span();
                            self.edits.push((span.lo.0, span.hi.0, candidate.3.clone()));
                            call.visit_children_with(self);
                            return;
                        }
                    }
                    if let (Some((receiver_key, receiver_source)), MemberProp::Ident(method)) =
                        (instance_receiver(&member.obj), &member.prop)
                    {
                        if let Some(class) = self.variables.get(&receiver_key) {
                            let has_callback = call.args.last().is_some_and(|argument| {
                                matches!(argument.expr.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                                    || matches!(argument.expr.as_ref(), Expr::Ident(identifier) if self.callbacks.contains(identifier.sym.as_str()))
                            });
                            let selected = self
                                .methods
                                .iter()
                                .filter(
                                    |(
                                        candidate_class,
                                        candidate_method,
                                        _,
                                        argument_count,
                                        candidate_callback,
                                        _,
                                    )| {
                                        candidate_class == class
                                            && candidate_method == method.sym.as_str()
                                            && *argument_count == call.args.len()
                                            && *candidate_callback == has_callback
                                    },
                                )
                                .filter_map(|candidate| {
                                    let mut score = 0u16;
                                    for (argument, declared) in
                                        call.args.iter().zip(candidate.5.iter())
                                    {
                                        if let Some(actual) = source_expr_type(
                                            argument.expr.as_ref(),
                                            &self.value_types,
                                            self.function_types,
                                        ) {
                                            score +=
                                                u16::from(overload_type_score(declared, &actual)?);
                                        }
                                    }
                                    Some((score, candidate))
                                })
                                .reduce(|best, candidate| {
                                    if candidate.0 > best.0 {
                                        candidate
                                    } else {
                                        best
                                    }
                                })
                                .map(|(_, candidate)| candidate);
                            if let Some((_, _, helper, _, _, _)) = selected {
                                let span = member.span();
                                self.edits.push((span.lo.0, span.hi.0, helper.clone()));
                                let insertion = if call.args.is_empty() {
                                    receiver_source
                                } else {
                                    format!("{receiver_source}, ")
                                };
                                self.edits.push((span.hi.0 + 1, span.hi.0 + 1, insertion));
                            }
                        }
                    }
                }
            }
            call.visit_children_with(self);
        }

        fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
            let mut setter_rewritten = false;
            if assignment.op == AssignOp::Assign {
                if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assignment.left {
                    if let (Some((qualifier, class)), Some(property)) = (
                        static_class_receiver(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        if let Some((_, _, _, helper, _)) = self.static_setters.iter().find(
                            |(
                                candidate_qualifier,
                                candidate_class,
                                candidate_property,
                                _,
                                declared,
                            )| {
                                candidate_class == &class
                                    && candidate_property == &property
                                    && qualifier
                                        .as_ref()
                                        .is_none_or(|qualifier| candidate_qualifier == qualifier)
                                    && source_expr_type(
                                        &assignment.right,
                                        &self.value_types,
                                        self.function_types,
                                    )
                                    .is_none_or(|actual| {
                                        overload_type_score(declared, &actual).is_some()
                                    })
                            },
                        ) {
                            let member_span = member.span();
                            let right_span = assignment.right.span();
                            let assignment_span = assignment.span();
                            self.edits
                                .push((member_span.lo.0, member_span.hi.0, helper.clone()));
                            self.edits
                                .push((member_span.hi.0, right_span.lo.0, "(".into()));
                            self.edits.push((
                                assignment_span.hi.0,
                                assignment_span.hi.0,
                                ")".into(),
                            ));
                            setter_rewritten = true;
                        }
                    }
                    if let (Some((receiver_key, receiver_source)), Some(property)) = (
                        instance_receiver(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        if let Some(class) = self.variables.get(&receiver_key) {
                            if let Some((_, _, helper, _)) =
                                self.setters.iter().find(
                                    |(candidate_class, candidate_property, _, declared)| {
                                        candidate_class == class
                                            && candidate_property == &property
                                            && source_expr_type(
                                                &assignment.right,
                                                &self.value_types,
                                                self.function_types,
                                            )
                                            .is_none_or(|actual| {
                                                overload_type_score(declared, &actual).is_some()
                                            })
                                    },
                                )
                            {
                                let member_span = member.span();
                                let right_span = assignment.right.span();
                                let assignment_span = assignment.span();
                                self.edits.push((
                                    member_span.lo.0,
                                    member_span.hi.0,
                                    helper.clone(),
                                ));
                                self.edits.push((
                                    member_span.hi.0,
                                    right_span.lo.0,
                                    format!("({receiver_source}, "),
                                ));
                                self.edits.push((
                                    assignment_span.hi.0,
                                    assignment_span.hi.0,
                                    ")".into(),
                                ));
                                setter_rewritten = true;
                            }
                        }
                    }
                }
            }
            if setter_rewritten {
                assignment.right.visit_with(self);
            } else {
                assignment.visit_children_with(self);
            }
            let inferred = (assignment.op == AssignOp::Assign)
                .then(|| {
                    source_expr_type(&assignment.right, &self.value_types, self.function_types)
                })
                .flatten();
            let assigned_class = (assignment.op == AssignOp::Assign)
                .then(|| source_instance_class(&assignment.right, self.classes, &self.variables))
                .flatten();
            let AssignTarget::Simple(target) = &assignment.left else {
                return;
            };
            match target {
                SimpleAssignTarget::Ident(binding) => {
                    if let Some(ty) = inferred {
                        self.value_types.insert(binding.id.sym.to_string(), ty);
                    } else {
                        self.value_types.remove(binding.id.sym.as_str());
                    }
                    if assignment.op == AssignOp::Assign {
                        invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                        if let Some(class) = assigned_class {
                            self.variables.insert(binding.id.sym.to_string(), class);
                        }
                    } else {
                        invalidate_instance_path(&mut self.variables, binding.id.sym.as_str());
                    }
                    if assignment.op == AssignOp::Assign
                        && matches!(assignment.right.as_ref(), Expr::Arrow(_) | Expr::Fn(_))
                    {
                        self.callbacks.insert(binding.id.sym.to_string());
                    } else {
                        self.callbacks.remove(binding.id.sym.as_str());
                    }
                }
                SimpleAssignTarget::Member(member) => {
                    let (root, path) = match member_assignment_path(member) {
                        Some(path) => path,
                        None => return,
                    };
                    let instance_key = instance_path_key(&root, &path);
                    invalidate_instance_path(&mut self.variables, &instance_key);
                    if let Some(class) = assigned_class {
                        self.variables.insert(instance_key, class);
                    }
                    let Some(value) = inferred else {
                        self.value_types.remove(&root);
                        return;
                    };
                    if let Some(object) = self.value_types.get_mut(&root) {
                        if !update_object_property_type(object, &path, value) {
                            self.value_types.remove(&root);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let (module, cm) = thaw_parser::parse_typescript_with_source_map(source)?;
    let mut function_types = FunctionTypeFinder::default();
    module.visit_with(&mut function_types);
    loop {
        let mut inferred = InferredFunctionTypeFinder {
            known: &function_types.types,
            additions: std::collections::HashMap::new(),
        };
        module.visit_with(&mut inferred);
        inferred
            .additions
            .retain(|name, _| !function_types.types.contains_key(name));
        if inferred.additions.is_empty() {
            break;
        }
        function_types.types.extend(inferred.additions);
    }
    let mut finder = Finder {
        classes,
        methods,
        static_methods,
        getters,
        setters,
        static_getters,
        static_setters,
        variables: std::collections::HashMap::new(),
        value_types: std::collections::HashMap::new(),
        function_types: &function_types.types,
        callbacks: std::collections::HashSet::new(),
        edits: Vec::new(),
        switch_break_depth: 0,
        switch_break_exits: Vec::new(),
    };
    module.visit_with(&mut finder);
    let mut edits = finder
        .edits
        .into_iter()
        .map(|(lo, hi, replacement)| {
            let lo = cm
                .lookup_byte_offset(thaw_parser::common::BytePos(lo))
                .pos
                .0 as usize;
            let hi = cm
                .lookup_byte_offset(thaw_parser::common::BytePos(hi))
                .pos
                .0 as usize;
            (lo, hi, replacement)
        })
        .collect::<Vec<_>>();
    edits.sort_by_key(|(lo, hi, _)| (*lo, *hi));
    let mut output = source.to_string();
    for (lo, hi, replacement) in edits.into_iter().rev() {
        output.replace_range(lo..hi, &replacement);
    }
    Ok(output)
}
