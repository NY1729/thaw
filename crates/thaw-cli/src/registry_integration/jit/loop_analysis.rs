macro_rules! jit_loop_analysis {
    () => {
    fn collect_loop_callable_assignments(
        statement: &Stmt,
        context: &InlineContext<'_>,
        output: &mut std::collections::HashMap<String, std::collections::BTreeSet<String>>,
    ) {
        match statement {
            Stmt::Expr(statement) => {
                let Expr::Assign(assignment) = statement.expr.as_ref() else {
                    return;
                };
                if assignment.op != AssignOp::Assign {
                    return;
                }
                let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
                    return;
                };
                if let Some(helpers) = finite_callable_names(assignment.right.as_ref()).or_else(|| {
                    callable_member_helpers(assignment.right.as_ref(), context)
                }) {
                    if helpers
                        .iter()
                        .all(|helper| context.helpers.contains_key(helper))
                    {
                        output
                            .entry(name.id.sym.to_string())
                            .or_default()
                            .extend(helpers);
                    }
                }
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_loop_callable_assignments(statement, context, output);
                }
            }
            Stmt::If(statement) => {
                collect_loop_callable_assignments(statement.cons.as_ref(), context, output);
                if let Some(alternate) = statement.alt.as_deref() {
                    collect_loop_callable_assignments(alternate, context, output);
                }
            }
            Stmt::While(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::DoWhile(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::For(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::ForOf(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::ForIn(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::Switch(statement) => {
                for case in &statement.cases {
                    for statement in &case.cons {
                        collect_loop_callable_assignments(statement, context, output);
                    }
                }
            }
            Stmt::Try(statement) => {
                for statement in &statement.block.stmts {
                    collect_loop_callable_assignments(statement, context, output);
                }
                if let Some(handler) = &statement.handler {
                    for statement in &handler.body.stmts {
                        collect_loop_callable_assignments(statement, context, output);
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_loop_callable_assignments(statement, context, output);
                    }
                }
            }
            Stmt::Labeled(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            _ => {}
        }
    }

    fn collect_local_value_assignments<'a>(
        statement: &'a Stmt,
        output: &mut std::collections::HashMap<String, Vec<&'a Expr>>,
    ) {
        match statement {
            Stmt::Expr(statement) => {
                let Expr::Assign(assignment) = statement.expr.as_ref() else {
                    return;
                };
                if assignment.op != AssignOp::Assign {
                    return;
                }
                let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
                    return;
                };
                output
                    .entry(name.id.sym.to_string())
                    .or_default()
                    .push(assignment.right.as_ref());
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_local_value_assignments(statement, output);
                }
            }
            Stmt::If(statement) => {
                collect_local_value_assignments(statement.cons.as_ref(), output);
                if let Some(alternate) = statement.alt.as_deref() {
                    collect_local_value_assignments(alternate, output);
                }
            }
            Stmt::While(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::DoWhile(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::For(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::ForIn(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::ForOf(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::Switch(statement) => {
                for case in &statement.cases {
                    for statement in &case.cons {
                        collect_local_value_assignments(statement, output);
                    }
                }
            }
            Stmt::Try(statement) => {
                for statement in &statement.block.stmts {
                    collect_local_value_assignments(statement, output);
                }
                if let Some(handler) = &statement.handler {
                    for statement in &handler.body.stmts {
                        collect_local_value_assignments(statement, output);
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_local_value_assignments(statement, output);
                    }
                }
            }
            Stmt::Labeled(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            _ => {}
        }
    }

    fn widened_local_kind(
        name: &Ident,
        initializer: &Expr,
        assignments: &[&Expr],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<(JitKind, std::collections::HashSet<JitKind>)> {
        let mut encoded = Vec::new();
        encode_expression(initializer, parameters, locals, context, &mut encoded)?;
        let mut kind = if boolean_literal(initializer) {
            JitKind::Boolean
        } else {
            jit_expression_kind(&encoded)?.0
        };
        let mut candidates: std::collections::HashSet<JitKind> = if kind == JitKind::Dynamic {
            encoded
                .iter()
                .filter_map(|token| match token.as_str() {
                    "tagnum" => Some(JitKind::Number),
                    "tagbool" => Some(JitKind::Boolean),
                    "tagstr" => Some(JitKind::String),
                    _ => None,
                })
                .collect()
        } else {
            std::iter::once(kind).collect()
        };
        if !matches!(
            kind,
            JitKind::Number | JitKind::Boolean | JitKind::String | JitKind::Dynamic
        ) {
            return None;
        }
        let mut probe_locals = locals.clone();
        for assignment in assignments {
            probe_locals.insert(
                name.sym.to_string(),
                vec![match kind {
                    JitKind::Number => "a0",
                    JitKind::Boolean => "b0",
                    JitKind::String => "s0",
                    JitKind::Dynamic => "ld0",
                    _ => return None,
                }
                .into()],
            );
            let mut encoded = Vec::new();
            encode_expression(
                assignment,
                parameters,
                &probe_locals,
                context,
                &mut encoded,
            )?;
            let assignment_kind = if boolean_literal(assignment) {
                JitKind::Boolean
            } else {
                jit_expression_kind(&encoded)?.0
            };
            if assignment_kind == JitKind::Dynamic {
                candidates.extend(encoded.iter().filter_map(|token| match token.as_str() {
                    "tagnum" => Some(JitKind::Number),
                    "tagbool" => Some(JitKind::Boolean),
                    "tagstr" => Some(JitKind::String),
                    _ => None,
                }));
            } else {
                candidates.insert(assignment_kind);
            }
            kind = merge_jit_kinds(kind, assignment_kind)?;
        }
        (kind == JitKind::Dynamic).then_some((kind, candidates))
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_loop_steps(
        steps: Vec<LocalStep<'_>>,
        loop_body: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &mut std::collections::HashMap<String, Vec<String>>,
        mutable: &mut std::collections::HashSet<String>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut candidates = std::collections::HashMap::new();
        collect_loop_callable_assignments(loop_body, context, &mut candidates);
        let mut value_assignments = std::collections::HashMap::new();
        collect_local_value_assignments(loop_body, &mut value_assignments);
        for step in steps {
            if let LocalStep::Declare {
                name,
                initializer,
                mutable: is_mutable,
            } = &step
            {
                let mut selection = Vec::new();
                let mut unused_local = 0usize;
                if let Some(alias) = encode_callable_member_snapshot(
                    initializer,
                    parameters,
                    locals,
                    context,
                    &mut unused_local,
                    &mut selection,
                ) {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    let (_, helpers) = callable_local_alias(&[alias])?;
                    let index = kinds.len();
                    output.extend(selection);
                    locals.insert(
                        name.sym.to_string(),
                        vec![format!(
                            "{CALLABLE_LOCAL_PREFIX}ln{index}:{}",
                            helpers.join("|")
                        )],
                    );
                    kinds.insert(name.sym.to_string(), JitKind::Number);
                    if *is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                    continue;
                }
                if let Some(initial_helpers) = finite_callable_names(initializer).filter(|helpers| {
                    helpers
                        .iter()
                        .all(|helper| context.helpers.contains_key(helper))
                }) {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    let mut helpers = initial_helpers;
                    for helper in candidates
                        .remove(name.sym.as_ref())
                        .into_iter()
                        .flatten()
                    {
                        if !helpers.contains(&helper) {
                            helpers.push(helper);
                        }
                    }
                    if helpers.len() == 1 {
                        locals.insert(
                            name.sym.to_string(),
                            vec![format!("{CALLABLE_ALIAS_PREFIX}{}", helpers[0])],
                        );
                    } else {
                        let index = kinds.len();
                        encode_callable_assignment_selection(
                            initializer,
                            &helpers,
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                        locals.insert(
                            name.sym.to_string(),
                            vec![format!(
                                "{CALLABLE_LOCAL_PREFIX}ln{index}:{}",
                                helpers.join("|")
                            )],
                        );
                        kinds.insert(name.sym.to_string(), JitKind::Number);
                    }
                    if *is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                    continue;
                }
            }
            if let LocalStep::Effect(expression) = &step {
                let mut effect = Vec::new();
                encode_expression(expression, parameters, locals, context, &mut effect)?;
                effect.push("drop".into());
                output.extend(effect);
                continue;
            }
            let forced_kind = match &step {
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable: true,
                } => value_assignments
                    .remove(name.sym.as_ref())
                    .and_then(|assignments| {
                        widened_local_kind(
                            name,
                            initializer,
                            &assignments,
                            parameters,
                            locals,
                            context,
                        )
                    }),
                _ => None,
            };
            encode_loop_declaration(
                step,
                forced_kind,
                parameters,
                locals,
                mutable,
                kinds,
                context,
                output,
            )?;
        }
        Some(())
    }

    fn collection_for_each_call(expression: &Expr) -> Option<(&Ident, &Expr)> {
        let Expr::Call(call) = expression else {
            return None;
        };
        let [callback] = call.args.as_slice() else {
            return None;
        };
        if callback.spread.is_some() {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let (Expr::Ident(receiver), MemberProp::Ident(method)) =
            (member.obj.as_ref(), &member.prop)
        else {
            return None;
        };
        (method.sym == *"forEach").then_some((receiver, callback.expr.as_ref()))
    }

    fn has_collection_for_each(steps: &[LocalStep<'_>]) -> bool {
        steps.iter().any(|step| {
            matches!(step, LocalStep::Effect(expression) if collection_for_each_call(expression).is_some())
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_collection_for_each(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (receiver, callback) = collection_for_each_call(expression)?;
        let map_prefix = context.map_locals.get(receiver.sym.as_ref()).cloned();
        if map_prefix.is_none() && !context.set_locals.contains(receiver.sym.as_ref()) {
            return None;
        }
        let callable = resolve_callable(callback, context.helpers)?;
        let (callback_parameters, steps, body) = callable_parts(callable)?;
        if callback_parameters.is_empty() || callback_parameters.len() > 3 {
            return None;
        }
        let callback_parameters = callback_parameters
            .iter()
            .map(|parameter| match parameter {
                Pat::Ident(name) => Some(&name.id),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        if callback_parameters.iter().any(|name| {
            parameters.contains_key(name.sym.as_ref()) || locals.contains_key(name.sym.as_ref())
        }) {
            return None;
        }

        let source_index = kinds.len();
        output.extend(locals.get(receiver.sym.as_ref())?.iter().cloned());
        output.extend(["dkeys".into(), "arrayhandle".into()]);
        kinds.insert(format!("\0foreach-source-{source_index}"), JitKind::Array);
        let source_local = format!("rsl{source_index}");
        let index = kinds.len();
        output.push(format!("c{:016x}", 0.0f64.to_bits()));
        kinds.insert(format!("\0foreach-index-{index}"), JitKind::Number);
        let index_local = format!("ln{index}");
        let key_index = kinds.len();
        encode_string("", output)?;
        kinds.insert(format!("\0foreach-key-{key_index}"), JitKind::String);
        let key_local = format!("ls{key_index}");

        let mut callback_locals = locals.clone();
        if let Some(collection) = callback_parameters.get(2) {
            callback_locals.insert(
                collection.sym.to_string(),
                locals.get(receiver.sym.as_ref())?.clone(),
            );
        }
        let value_binding = if let Some(prefix) = map_prefix {
            let value_index = kinds.len();
            let (kind, local) = match prefix.as_str() {
                "dn" => {
                    output.push(format!("c{:016x}", 0.0f64.to_bits()));
                    (JitKind::Number, format!("ln{value_index}"))
                }
                "db" => {
                    output.extend([
                        format!("c{:016x}", 0.0f64.to_bits()),
                        "asbool".into(),
                    ]);
                    (JitKind::Boolean, format!("lb{value_index}"))
                }
                "ds" => {
                    encode_string("", output)?;
                    (JitKind::String, format!("ls{value_index}"))
                }
                _ => return None,
            };
            kinds.insert(format!("\0foreach-value-{value_index}"), kind);
            callback_locals.insert(callback_parameters[0].sym.to_string(), vec![local]);
            if let Some(key) = callback_parameters.get(1) {
                callback_locals.insert(key.sym.to_string(), vec![key_local.clone()]);
            }
            Some((value_index, prefix))
        } else {
            callback_locals.insert(
                callback_parameters[0].sym.to_string(),
                vec![key_local.clone()],
            );
            if let Some(value) = callback_parameters.get(1) {
                callback_locals.insert(value.sym.to_string(), vec![key_local.clone()]);
            }
            None
        };

        fn pure_local(expression: &Expr) -> bool {
            match expression {
                Expr::Lit(_) | Expr::Ident(_) => true,
                Expr::Paren(expression) => pure_local(expression.expr.as_ref()),
                Expr::Unary(expression) => pure_local(expression.arg.as_ref()),
                Expr::Bin(expression) => {
                    pure_local(expression.left.as_ref()) && pure_local(expression.right.as_ref())
                }
                Expr::Cond(expression) => {
                    pure_local(expression.test.as_ref())
                        && pure_local(expression.cons.as_ref())
                        && pure_local(expression.alt.as_ref())
                }
                Expr::Member(member) => {
                    pure_local(member.obj.as_ref())
                        && match &member.prop {
                            MemberProp::Ident(_) | MemberProp::PrivateName(_) => true,
                            MemberProp::Computed(property) => pure_local(property.expr.as_ref()),
                        }
                }
                _ => false,
            }
        }
        let mut callback_effects = Vec::new();
        let mut callback_initializers = Vec::new();
        let mut callback_mutable = mutable.clone();
        let callback_mutations = steps
            .iter()
            .filter_map(|step| match step {
                LocalStep::Assign { name, .. } | LocalStep::Update { name, .. } => {
                    Some(name.sym.as_ref())
                }
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        let mut declarations_done = false;
        for step in steps {
            match step {
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable,
                } if !declarations_done
                    && (!mutable || !callback_mutations.contains(name.sym.as_ref())) =>
                {
                    if callback_locals.contains_key(name.sym.as_ref()) || !pure_local(initializer) {
                        return None;
                    }
                    let mut encoded = Vec::new();
                    encode_expression(
                        initializer,
                        parameters,
                        &callback_locals,
                        context,
                        &mut encoded,
                    )?;
                    jit_expression_kind(&encoded)?;
                    callback_locals.insert(name.sym.to_string(), encoded);
                }
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable: true,
                } if !declarations_done && callback_mutations.contains(name.sym.as_ref()) => {
                    if callback_locals.contains_key(name.sym.as_ref()) || !pure_local(initializer) {
                        return None;
                    }
                    let mut encoded = Vec::new();
                    encode_expression(
                        initializer,
                        parameters,
                        &callback_locals,
                        context,
                        &mut encoded,
                    )?;
                    let kind = jit_expression_kind(&encoded)?.0;
                    let index = kinds.len();
                    let local = match kind {
                        JitKind::Number => {
                            output.push(format!("c{:016x}", 0.0f64.to_bits()));
                            format!("ln{index}")
                        }
                        JitKind::Boolean => {
                            output.extend([
                                format!("c{:016x}", 0.0f64.to_bits()),
                                "asbool".into(),
                            ]);
                            format!("lb{index}")
                        }
                        JitKind::String => {
                            encode_string("", output)?;
                            format!("ls{index}")
                        }
                        _ => return None,
                    };
                    kinds.insert(name.sym.to_string(), kind);
                    callback_locals.insert(name.sym.to_string(), vec![local]);
                    callback_mutable.insert(name.sym.to_string());
                    callback_initializers.push((index, kind, encoded, name.sym.to_string()));
                }
                LocalStep::Declare { .. }
                | LocalStep::DestructureArray { .. }
                | LocalStep::DestructureObject { .. } => return None,
                effect => {
                    declarations_done = true;
                    callback_effects.push(effect);
                }
            }
        }

        output.extend([
            "loop".into(),
            index_local.clone(),
            source_local.clone(),
            "arraylen".into(),
            "<".into(),
            "while".into(),
            source_local,
            index_local.clone(),
            "rsget".into(),
            format!("setl{key_index}"),
        ]);
        if let Some((value_index, prefix)) = value_binding {
            output.extend(locals.get(receiver.sym.as_ref())?.iter().cloned());
            output.extend([
                key_local,
                format!("{prefix}get"),
                format!("setl{value_index}"),
            ]);
        }
        for (index, kind, initializer, _) in &callback_initializers {
            output.extend(initializer.iter().cloned());
            if kind == &JitKind::Boolean {
                output.push("asbool".into());
            }
            output.push(format!("setl{index}"));
        }
        for effect in callback_effects {
            match effect {
                LocalStep::Assign {
                    name,
                    operation,
                    value,
                } => encode_local_assignment(
                    name,
                    operation,
                    value,
                    parameters,
                    &callback_locals,
                    &callback_mutable,
                    kinds,
                    context,
                    output,
                )?,
                LocalStep::Update { name, operation } => {
                    encode_local_update(
                        name,
                        operation,
                        &callback_locals,
                        &callback_mutable,
                        kinds,
                        output,
                    )?
                }
                LocalStep::Effect(expression) => encode_loop_expression(
                    expression,
                    parameters,
                    &callback_locals,
                    &callback_mutable,
                    kinds,
                    context,
                    output,
                )?,
                _ => return None,
            }
        }
        match body {
            NumericBody::Expression(expression) => encode_loop_expression(
                expression,
                parameters,
                &callback_locals,
                &callback_mutable,
                kinds,
                context,
                output,
            )?,
            NumericBody::Statements(statements) => {
                for statement in statements {
                    match statement {
                        Stmt::Expr(_) => encode_loop_effects(
                            statement,
                            parameters,
                            &callback_locals,
                            &callback_mutable,
                            (kinds, root_loop_control(), &[]),
                            context,
                            output,
                        )?,
                        Stmt::Return(statement) => {
                            if let Some(value) = statement.arg.as_deref() {
                                encode_loop_expression(
                                    value,
                                    parameters,
                                    &callback_locals,
                                    &callback_mutable,
                                    kinds,
                                context,
                                output,
                            )?;
                        }
                        }
                        _ => return None,
                    }
                }
            }
        }
        output.extend([
            "looptail".into(),
            index_local,
            format!("c{:016x}", 1.0f64.to_bits()),
            "+".into(),
            format!("setl{index}"),
            "loopend".into(),
        ]);
        for (index, kind, _, name) in callback_initializers {
            kinds.remove(&name);
            kinds.insert(format!("\0foreach-local-{index}"), kind);
        }
        Some(())
    }

    fn encode_while_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::While(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        output.push("loop".into());
        output.push("looptail".into());
        encode_condition(
            loop_statement.test.as_ref(), parameters, &locals, context, output,
        )?;
        output.push("while".into());
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("loopend".into());
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_for_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::For(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        match loop_statement.init.as_ref() {
            Some(VarDeclOrExpr::VarDecl(declaration)) => {
                for declarator in &declaration.decls {
                    let Pat::Ident(name) = &declarator.name else {
                        return None;
                    };
                    encode_loop_declaration(
                        LocalStep::Declare {
                            name: &name.id,
                            initializer: declarator.init.as_deref()?,
                            mutable: declaration.kind != VarDeclKind::Const,
                        },
                        None,
                        parameters,
                        &mut locals,
                        &mut mutable,
                        &mut kinds,
                        context,
                        output,
                    )?;
                }
            }
            Some(VarDeclOrExpr::Expr(initializer)) => encode_loop_expression(
                initializer.as_ref(),
                parameters,
                &locals,
                &mutable,
                &kinds,
                context,
                output,
            )?,
            None => {}
        }
        output.push("loop".into());
        if let Some(test) = loop_statement.test.as_deref() {
            encode_condition(test, parameters, &locals, context, output)?;
        } else {
            output.extend([
                format!("c{:016x}", 1.0f64.to_bits()),
                "asbool".into(),
            ]);
        }
        output.push("while".into());
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("looptail".into());
        if let Some(update) = loop_statement.update.as_deref() {
            encode_loop_expression(
                update,
                parameters,
                &locals,
                &mutable,
                &kinds,
                context,
                output,
            )?;
        }
        output.push("loopend".into());
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_do_while_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::DoWhile(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        output.push("loop".into());
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("looptail".into());
        encode_condition(
            loop_statement.test.as_ref(), parameters, &locals, context, output,
        )?;
        output.push("while".into());
        output.push("loopend".into());
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_for_of_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::ForOf(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        if loop_statement.is_await {
            return None;
        }
        let mut source = Vec::new();
        let map_iteration = match loop_statement.right.as_ref() {
            Expr::Ident(name) => context
                .map_locals
                .get(name.sym.as_ref())
                .and_then(|prefix| {
                    locals
                        .get(name.sym.as_ref())
                        .cloned()
                        .map(|source| (prefix.clone(), source))
                }),
            _ => None,
        };
        encode_expression(
            loop_statement.right.as_ref(), parameters, &locals, context, &mut source,
        )?;
        // `for (const x of set)` iterates the Set's elements -- the keys
        // of the string-keyed dictionary it's modeled as.
        if jit_expression_kind(&source)?.0 == JitKind::Dictionary {
            let is_set = matches!(loop_statement.right.as_ref(), Expr::Ident(name)
                if context.set_locals.contains(name.sym.as_ref()));
            if !is_set && map_iteration.is_none() {
                return None;
            }
            source.push("dkeys".into());
        }
        if source.len() == 1 {
            if let Some(untag) = jit_typed_array_union_untag(&source[0]) {
                source.push(untag.into());
            }
        }
        let dynamic_array = (source.len() == 1 && jit_dynamic_array_argument(&source[0]))
            || jit_dynamic_array_result(&source);
        match jit_expression_kind(&source)?.0 {
            JitKind::String => source.push("strarray".into()),
            JitKind::Array => {}
            JitKind::Dynamic if dynamic_array => {}
            _ => return None,
        }
        let array = if dynamic_array {
            "dynamic"
        } else {
            array_prefix(&source)?
        };
        let element_kind = match array {
            "rn" => JitKind::Number,
            "rb" => JitKind::Boolean,
            "rs" => JitKind::String,
            "dynamic" => JitKind::Dynamic,
            _ => return None,
        };
        let source_index = kinds.len();
        output.extend(source);
        kinds.insert(
            format!("\0forof-source-{source_index}"),
            if dynamic_array {
                JitKind::Dynamic
            } else {
                JitKind::Array
            },
        );
        let source_local = if dynamic_array {
            format!("ld{source_index}")
        } else {
            format!("{array}l{source_index}")
        };

        let index = kinds.len();
        output.push(format!("c{:016x}", 0.0f64.to_bits()));
        kinds.insert(format!("\0forof-index-{index}"), JitKind::Number);
        let index_local = format!("ln{index}");

        let mut map_value_binding = None;
        let element_index = match &loop_statement.left {
            ForHead::VarDecl(declaration) => {
                let [declarator] = declaration.decls.as_slice() else {
                    return None;
                };
                if let Some((map_prefix, map_source)) = &map_iteration {
                    let Pat::Array(pattern) = &declarator.name else {
                        return None;
                    };
                    let [Some(Pat::Ident(key)), Some(Pat::Ident(value))] =
                        pattern.elems.as_slice()
                    else {
                        return None;
                    };
                    if declarator.init.is_some()
                        || parameters.contains_key(key.id.sym.as_ref())
                        || parameters.contains_key(value.id.sym.as_ref())
                        || locals.contains_key(key.id.sym.as_ref())
                        || locals.contains_key(value.id.sym.as_ref())
                    {
                        return None;
                    }
                    encode_string("", output)?;
                    let key_index = kinds.len();
                    let key_local = format!("ls{key_index}");
                    locals.insert(key.id.sym.to_string(), vec![key_local.clone()]);
                    kinds.insert(key.id.sym.to_string(), JitKind::String);
                    match map_prefix.as_str() {
                        "dn" => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                        "db" => {
                            output.push(format!("c{:016x}", 0.0f64.to_bits()));
                            output.push("asbool".into());
                        }
                        "ds" => encode_string("", output)?,
                        _ => return None,
                    }
                    let value_index = kinds.len();
                    let (value_kind, value_local) = match map_prefix.as_str() {
                        "dn" => (JitKind::Number, format!("ln{value_index}")),
                        "db" => (JitKind::Boolean, format!("lb{value_index}")),
                        "ds" => (JitKind::String, format!("ls{value_index}")),
                        _ => return None,
                    };
                    locals.insert(value.id.sym.to_string(), vec![value_local]);
                    kinds.insert(value.id.sym.to_string(), value_kind);
                    if declaration.kind != VarDeclKind::Const {
                        mutable.insert(key.id.sym.to_string());
                        mutable.insert(value.id.sym.to_string());
                    }
                    map_value_binding = Some((
                        value_index,
                        map_source.clone(),
                        key_local,
                        map_prefix.clone(),
                    ));
                    key_index
                } else {
                    let Pat::Ident(name) = &declarator.name else {
                        return None;
                    };
                    if declarator.init.is_some()
                        || parameters.contains_key(name.id.sym.as_ref())
                        || locals.contains_key(name.id.sym.as_ref())
                    {
                        return None;
                    }
                    match element_kind {
                        JitKind::String => encode_string("", output)?,
                        JitKind::Number | JitKind::Boolean => {
                            output.push(format!("c{:016x}", 0.0f64.to_bits()));
                            if element_kind == JitKind::Boolean {
                                output.push("asbool".into());
                            }
                        }
                        JitKind::Dynamic => {
                            output.push(format!("c{:016x}", 0.0f64.to_bits()));
                            output.push("tagnum".into());
                        }
                        _ => return None,
                    }
                    let element_index = kinds.len();
                    let prefix = match element_kind {
                        JitKind::Number => "ln",
                        JitKind::Boolean => "lb",
                        JitKind::String => "ls",
                        JitKind::Dynamic => "ld",
                        _ => return None,
                    };
                    let element_local = format!("{prefix}{element_index}");
                    locals.insert(name.id.sym.to_string(), vec![element_local.clone()]);
                    kinds.insert(name.id.sym.to_string(), element_kind);
                    if declaration.kind != VarDeclKind::Const {
                        mutable.insert(name.id.sym.to_string());
                    }
                    element_index
                }
            }
            ForHead::Pat(pattern) => {
                let Pat::Ident(name) = pattern.as_ref() else {
                    return None;
                };
                if !mutable.contains(name.id.sym.as_ref())
                    || kinds.get(name.id.sym.as_ref())? != &element_kind
                {
                    return None;
                }
                loop_local_index(locals.get(name.id.sym.as_ref())?.first()?)?
            }
            ForHead::UsingDecl(_) => return None,
        };
        output.push("loop".into());
        output.extend([
            index_local.clone(),
            source_local.clone(),
            if dynamic_array {
                "dynarraylen".into()
            } else {
                "arraylen".into()
            },
            "<".into(),
        ]);
        output.push("while".into());
        output.extend([source_local.clone(), index_local.clone()]);
        output.push(if dynamic_array {
            "dynarrayat".into()
        } else {
            format!("{array}get")
        });
        output.push(format!("setl{element_index}"));
        if let Some((value_index, map_source, key_local, map_prefix)) = map_value_binding {
            output.extend(map_source);
            output.push(key_local);
            output.push(format!("{map_prefix}get"));
            output.push(format!("setl{value_index}"));
        }
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("looptail".into());
        output.extend([
            index_local,
            format!("c{:016x}", 1.0f64.to_bits()),
            "+".into(),
            format!("setl{index}"),
            "loopend".into(),
        ]);
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }
    fn encode_for_in_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::ForIn(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        encode_for_in_loop(
            loop_statement,
            parameters,
            &mut locals,
            &mut mutable,
            (&mut kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    };
}
