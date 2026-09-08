macro_rules! jit_loop_aliases {
    () => {
    fn encode_callable_alias_flow(
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let [statement @ (Stmt::Block(_)
            | Stmt::If(_)
            | Stmt::Switch(_)
            | Stmt::Try(_)
            | Stmt::While(_)
            | Stmt::DoWhile(_)
            | Stmt::For(_)
            | Stmt::ForOf(_)
            | Stmt::ForIn(_)
            | Stmt::Labeled(_)), rest @ ..] = statements
        {
            let kinds = callable_runtime_kinds(locals)?;
            if !kinds.is_empty() {
                let mut control_flow = Vec::new();
                if encode_loop_effects(
                    statement,
                    parameters,
                    locals,
                    mutable,
                    (&kinds, root_loop_control(), &[]),
                    context,
                    &mut control_flow,
                )
                .is_some()
                {
                    let mut tail = Vec::new();
                    encode_callable_alias_flow(
                        rest, parameters, locals, mutable, context, &mut tail,
                    )?;
                    output.extend(control_flow);
                    output.extend(tail);
                    return Some(());
                }
            }
        }
        let [Stmt::If(branch), rest @ ..] = statements else {
            return encode_returning_statements(
                statements,
                parameters,
                locals,
                context,
                output,
            );
        };
        if rest.is_empty() {
            return encode_returning_statements(
                statements,
                parameters,
                locals,
                context,
                output,
            );
        }
        let Some((name, consequent)) =
            callable_alias_assignment(branch.cons.as_ref(), locals, context)
        else {
            return encode_returning_statements(
                statements,
                parameters,
                locals,
                context,
                output,
            );
        };
        if !mutable.contains(&name) {
            return None;
        }
        let initial = locals
            .get(&name)?
            .as_slice()
            .first()?
            .strip_prefix(CALLABLE_ALIAS_PREFIX)?
            .to_owned();
        let alternate = if let Some(statement) = branch.alt.as_deref() {
            let (alternate_name, alternate) =
                callable_alias_assignment(statement, locals, context)?;
            (alternate_name == name).then_some(alternate)?
        } else {
            initial
        };
        encode_condition(
            branch.test.as_ref(),
            parameters,
            locals,
            context,
            output,
        )?;
        output.push("if".into());
        let mut selected = locals.clone();
        selected.insert(
            name.clone(),
            vec![format!("{CALLABLE_ALIAS_PREFIX}{consequent}")],
        );
        encode_callable_alias_flow(
            rest,
            parameters,
            &selected,
            mutable,
            context,
            output,
        )?;
        output.push("else".into());
        selected.insert(
            name,
            vec![format!("{CALLABLE_ALIAS_PREFIX}{alternate}")],
        );
        encode_callable_alias_flow(
            rest,
            parameters,
            &selected,
            mutable,
            context,
            output,
        )?;
        output.push("end".into());
        Some(())
    }

    fn callable_runtime_kinds(
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<std::collections::HashMap<String, JitKind>> {
        let mut indexed = Vec::new();
        for (name, tokens) in locals {
            let (local, kind) = if let Some((local, _)) = callable_local_alias(tokens) {
                (local, JitKind::Number)
            } else {
                let Some(local) = tokens.first() else {
                    continue;
                };
                let Some(kind) = runtime_local_kind(local) else {
                    continue;
                };
                (local.clone(), kind)
            };
            let index = loop_local_index(&local)?;
            if indexed.len() <= index {
                indexed.resize(index + 1, None);
            }
            if indexed[index].replace((name.clone(), kind)).is_some() {
                return None;
            }
        }
        indexed.into_iter().collect()
    }

    fn runtime_local_kind(local: &str) -> Option<JitKind> {
        let prefix = local.get(..local.find(|character: char| character.is_ascii_digit())?)?;
        match prefix {
            "ln" => Some(JitKind::Number),
            "lb" => Some(JitKind::Boolean),
            "ls" => Some(JitKind::String),
            "ld" => Some(JitKind::Dynamic),
            "rnl" | "rbl" | "rsl" => Some(JitKind::Array),
            "dnl" | "dbl" | "dsl" => Some(JitKind::Dictionary),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_local_reassignment(
        expression: &Expr,
        local: &str,
        helpers: &[String],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut selection = Vec::new();
        let mut unused_local = 0usize;
        if let Some(alias) = encode_callable_member_snapshot(
            expression,
            parameters,
            locals,
            context,
            &mut unused_local,
            &mut selection,
        ) {
            let (_, selected_helpers) = callable_local_alias(&[alias])?;
            if helpers != selected_helpers {
                encode_callable_selection_remap(&selected_helpers, helpers, &mut selection)?;
            }
        } else {
            encode_callable_assignment_selection(
                expression,
                helpers,
                parameters,
                locals,
                context,
                &mut selection,
            )?;
        }
        output.extend(selection);
        output.push(format!("setl{}", loop_local_index(local)?));
        Some(())
    }

    fn encode_callable_selection_remap(
        source_helpers: &[String],
        target_helpers: &[String],
        output: &mut Vec<String>,
    ) -> Option<()> {
        if target_helpers.starts_with(source_helpers) {
            return Some(());
        }
        let branches = source_helpers
            .iter()
            .map(|source| {
                let target = target_helpers.iter().position(|helper| helper == source)?;
                Some(vec![format!("c{:016x}", (target as f64).to_bits())])
            })
            .collect::<Option<Vec<_>>>()?;
        encode_dynamic_callable_branches(&branches, 0, "dup", output)?;
        output.push("nip".into());
        Some(())
    }

    fn callable_alias_assignment(
        statement: &Stmt,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<(String, String)> {
        if let Stmt::Block(block) = statement {
            let [statement] = block.stmts.as_slice() else {
                return None;
            };
            return callable_alias_assignment(statement, locals, context);
        }
        let Stmt::Expr(statement) = statement else {
            return None;
        };
        let Expr::Assign(assignment) = statement.expr.as_ref() else {
            return None;
        };
        if assignment.op != AssignOp::Assign {
            return None;
        }
        let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
            return None;
        };
        Some((
            name.id.sym.to_string(),
            callable_alias(assignment.right.as_ref(), locals, context)?,
        ))
    }
    };
}

