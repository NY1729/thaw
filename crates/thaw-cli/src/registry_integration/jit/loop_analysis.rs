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
        encode_expression(
            loop_statement.right.as_ref(), parameters, &locals, context, &mut source,
        )?;
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

        let element_index = match &loop_statement.left {
            ForHead::VarDecl(declaration) => {
                let [declarator] = declaration.decls.as_slice() else {
                    return None;
                };
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

