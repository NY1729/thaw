macro_rules! jit_loop_expressions {
    () => {
    fn encode_loop_effects(
        statement: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        control: (
            &std::collections::HashMap<String, JitKind>,
            LoopControl,
            &[&thaw_parser::ast::BlockStmt],
        ),
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (kinds, loop_control, finalizers) = control;

        fn encode_finalizers(
            finalizers: &[&thaw_parser::ast::BlockStmt],
            from: usize,
            scope: LoopScope<'_>,
            loop_control: LoopControl,
            context: &mut InlineContext<'_>,
            output: &mut Vec<String>,
        ) -> Option<()> {
            let (parameters, locals, mutable, kinds) = scope;
            for index in (from..finalizers.len()).rev() {
                encode_loop_effects(
                    &Stmt::Block(finalizers[index].clone()),
                    parameters,
                    locals,
                    mutable,
                    (kinds, loop_control, &finalizers[..index]),
                    context,
                    output,
                )?;
            }
            Some(())
        }
        if let Stmt::Labeled(labeled) = statement {
            let target = context.loop_depth;
            context
                .loop_labels
                .push((labeled.label.sym.to_string(), target));
            let result = encode_loop_effects(
                labeled.body.as_ref(),
                parameters,
                locals,
                mutable,
                (kinds, loop_control, finalizers),
                context,
                output,
            );
            context.loop_labels.pop();
            return result;
        }
        if let Stmt::Block(block) = statement {
            let mut block_locals = locals.clone();
            let mut block_mutable = mutable.clone();
            let mut block_kinds = kinds.clone();
            let mut declared = std::collections::HashSet::new();
            for statement in &block.stmts {
                if let Stmt::Decl(Decl::Var(declaration)) = statement {
                    for declarator in &declaration.decls {
                        let Pat::Ident(name) = &declarator.name else {
                            return None;
                        };
                        if !declared.insert(name.id.sym.to_string()) {
                            return None;
                        }
                        block_locals.remove(name.id.sym.as_ref());
                        block_mutable.remove(name.id.sym.as_ref());
                        if let Some(kind) = block_kinds.remove(name.id.sym.as_ref()) {
                            let slot = block_kinds.len();
                            block_kinds.insert(format!("\0shadow-{slot}"), kind);
                        }
                        let start = output.len();
                        encode_loop_declaration(
                            LocalStep::Declare {
                                name: &name.id,
                                initializer: declarator.init.as_deref()?,
                                mutable: declaration.kind != VarDeclKind::Const,
                            },
                            None,
                            parameters,
                            &mut block_locals,
                            &mut block_mutable,
                            &mut block_kinds,
                            context,
                            output,
                        )?;
                        if loop_control.catch_active {
                            insert_jit_error_checks(output, start);
                        }
                    }
                    continue;
                }
                encode_loop_effects(
                    statement,
                    parameters,
                    &block_locals,
                    &block_mutable,
                    (&block_kinds, loop_control, finalizers),
                    context,
                    output,
                )?;
            }
            output.extend(std::iter::repeat_n(
                "drop".into(),
                block_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        if let Stmt::If(branch) = statement {
            if let Some((name, tag_index, value_index, selected, alternate_kind, probe)) =
                dynamic_catch_narrowing(branch.test.as_ref(), locals)
            {
                output.extend([
                    format!("ln{tag_index}"),
                    format!("c{:016x}", f64::from(caught_throw_tag(&selected)?).to_bits()),
                    "==".into(),
                ]);
                match probe {
                    CatchProbe::Tag => {}
                    CatchProbe::ArrayIndex(index) => output.extend([
                        "if".into(),
                        format!("{}{value_index}", selected.prefix),
                        "arraylen".into(),
                        format!("c{:016x}", f64::from(index).to_bits()),
                        ">".into(),
                        "else".into(),
                        "c0000000000000000".into(),
                        "end".into(),
                    ]),
                    CatchProbe::DictionaryKey(key) => {
                        output.push("if".into());
                        encode_string(&key, output)?;
                        output.extend([
                            format!("{}{value_index}", selected.prefix),
                            "din".into(),
                            "else".into(),
                            "c0000000000000000".into(),
                            "end".into(),
                        ]);
                    }
                }
                output.push("guard".into());
                let mut selected_locals = locals.clone();
                selected_locals.insert(
                    name.clone(),
                    vec![format!("{}{value_index}", selected.prefix)],
                );
                let mut selected_kinds = kinds.clone();
                selected_kinds.insert(name.clone(), selected.kind);
                encode_loop_effects(
                    branch.cons.as_ref(),
                    parameters,
                    &selected_locals,
                    mutable,
                    (&selected_kinds, loop_control, finalizers),
                    context,
                    output,
                )?;
                if let Some(alternate) = branch.alt.as_deref() {
                    let alternate_kind = alternate_kind?;
                    output.push("guardelse".into());
                    let mut alternate_locals = locals.clone();
                    alternate_locals.insert(
                        name.clone(),
                        vec![format!("{}{value_index}", alternate_kind.prefix)],
                    );
                    let mut alternate_kinds = kinds.clone();
                    alternate_kinds.insert(name, alternate_kind.kind);
                    encode_loop_effects(
                        alternate,
                        parameters,
                        &alternate_locals,
                        mutable,
                        (&alternate_kinds, loop_control, finalizers),
                        context,
                        output,
                    )?;
                }
                output.push("guardend".into());
                return Some(());
            }
            let start = output.len();
            encode_condition(branch.test.as_ref(), parameters, locals, context, output)?;
            if loop_control.catch_active {
                insert_jit_error_checks(output, start);
            }
            if let Some((consequent_locals, alternate_locals)) = narrowed_locals(
                branch.test.as_ref(),
                parameters,
                locals,
                context.helpers,
            ) {
                output.push("guard".into());
                encode_loop_effects(
                    branch.cons.as_ref(),
                    parameters,
                    &consequent_locals,
                    mutable,
                    (kinds, loop_control, finalizers),
                    context,
                    output,
                )?;
                if let Some(alternate) = branch.alt.as_deref() {
                    output.push("guardelse".into());
                    encode_loop_effects(
                        alternate,
                        parameters,
                        &alternate_locals,
                        mutable,
                        (kinds, loop_control, finalizers),
                        context,
                        output,
                    )?;
                }
                output.push("guardend".into());
                return Some(());
            }
            output.push("guard".into());
            encode_loop_effects(
                branch.cons.as_ref(), parameters, locals, mutable, (kinds, loop_control, finalizers), context, output,
            )?;
            if let Some(alternate) = branch.alt.as_deref() {
                output.push("guardelse".into());
                encode_loop_effects(
                    alternate, parameters, locals, mutable, (kinds, loop_control, finalizers), context, output,
                )?;
            }
            output.push("guardend".into());
            return Some(());
        }
        if matches!(statement, Stmt::Break(statement) if statement.label.is_none()) {
            encode_finalizers(
                finalizers,
                loop_control.break_finalizer_depth,
                (parameters, locals, mutable, kinds),
                loop_control,
                context,
                output,
            )?;
            output.push(match loop_control.break_target {
                BreakTarget::Loop => "break",
                BreakTarget::Switch => "switchbreak",
            }.into());
            return Some(());
        }
        if let Stmt::Break(statement) = statement {
            let label = statement.label.as_ref()?.sym.to_string();
            let target = context
                .loop_labels
                .iter()
                .rev()
                .find(|(name, _)| name == &label)
                .map(|(_, depth)| *depth)?;
            let distance = context
                .loop_depth
                .checked_sub(target.checked_add(1)?)?;
            encode_finalizers(
                finalizers,
                loop_control.break_finalizer_depth,
                (parameters, locals, mutable, kinds),
                loop_control,
                context,
                output,
            )?;
            output.push(format!("break{distance}"));
            return Some(());
        }
        if matches!(statement, Stmt::Continue(statement) if statement.label.is_none()) {
            encode_finalizers(
                finalizers,
                loop_control.continue_finalizer_depth,
                (parameters, locals, mutable, kinds),
                loop_control,
                context,
                output,
            )?;
            output.push("continue".into());
            return Some(());
        }
        if let Stmt::Continue(statement) = statement {
            let label = statement.label.as_ref()?.sym.to_string();
            let target = context
                .loop_labels
                .iter()
                .rev()
                .find(|(name, _)| name == &label)
                .map(|(_, depth)| *depth)?;
            let distance = context
                .loop_depth
                .checked_sub(target.checked_add(1)?)?;
            encode_finalizers(
                finalizers,
                loop_control.continue_finalizer_depth,
                (parameters, locals, mutable, kinds),
                loop_control,
                context,
                output,
            )?;
            output.push(format!("continue{distance}"));
            return Some(());
        }
        if let Stmt::Switch(statement) = statement {
            return encode_loop_switch(
                statement,
                parameters,
                locals,
                mutable,
                (kinds, loop_control, finalizers),
                context,
                output,
            );
        }
        if let Stmt::Try(statement) = statement {
            let mut nested_finalizers = finalizers.to_vec();
            if let Some(finalizer) = &statement.finalizer {
                nested_finalizers.push(finalizer);
            }
            if let Some(handler) = &statement.handler {
                let mut caught_kinds = Vec::new();
                for statement in &statement.block.stmts {
                    collect_caught_throw_kind(
                        statement,
                        parameters,
                        locals,
                        context,
                        &mut caught_kinds,
                    )?;
                }
                if caught_kinds.is_empty() {
                    caught_kinds.push(CaughtThrowKind {
                        kind: JitKind::String,
                        prefix: "ls",
                    });
                }
                let tagged = caught_kinds.len() > 1;
                output.push(if tagged { "trystarttag" } else { "trystart" }.into());
                let caught_control = LoopControl {
                    throw_finalizer_depth: nested_finalizers.len(),
                    catch_active: true,
                    catch_tagged: tagged,
                    ..loop_control
                };
                encode_loop_effects(
                    &Stmt::Block(statement.block.clone()),
                    parameters,
                    locals,
                    mutable,
                    (kinds, caught_control, &nested_finalizers),
                    context,
                    output,
                )?;
                output.push("catch".into());

                let mut catch_locals = locals.clone();
                let mut handler_kinds = kinds.clone();
                if let Some(parameter) = &handler.param {
                    let Pat::Ident(identifier) = parameter else {
                        return None;
                    };
                    if parameters.contains_key(identifier.id.sym.as_ref())
                        || locals.contains_key(identifier.id.sym.as_ref())
                    {
                        return None;
                    }
                    let index = handler_kinds.len();
                    if tagged {
                        let variants = caught_kinds
                            .iter()
                            .map(|kind| {
                                Some(format!(
                                    "{}={}",
                                    caught_throw_tag(kind)?,
                                    kind.prefix
                                ))
                            })
                            .collect::<Option<Vec<_>>>()?
                            .join("|");
                        catch_locals.insert(
                            identifier.id.sym.to_string(),
                            vec![format!("x{index}:{}:{variants}", index + 1)],
                        );
                        handler_kinds.insert(
                            format!("\0catch-tag-{index}"),
                            JitKind::Number,
                        );
                        handler_kinds.insert(identifier.id.sym.to_string(), JitKind::Number);
                    } else {
                        let caught_kind = &caught_kinds[0];
                        catch_locals.insert(
                            identifier.id.sym.to_string(),
                            vec![format!("{}{index}", caught_kind.prefix)],
                        );
                        handler_kinds.insert(identifier.id.sym.to_string(), caught_kind.kind);
                    }
                } else {
                    let slots = if tagged { 2 } else { 1 };
                    for slot in 0..slots {
                        handler_kinds.insert(
                            format!("\0catch-{}-{slot}", handler_kinds.len()),
                            JitKind::Number,
                        );
                    }
                }
                encode_loop_effects(
                    &Stmt::Block(handler.body.clone()),
                    parameters,
                    &catch_locals,
                    mutable,
                    (&handler_kinds, loop_control, &nested_finalizers),
                    context,
                    output,
                )?;
                output.extend(std::iter::repeat_n("drop".into(), if tagged { 2 } else { 1 }));
                output.push("tryend".into());
                if let Some(finalizer) = &statement.finalizer {
                    return encode_loop_effects(
                        &Stmt::Block(finalizer.clone()),
                        parameters,
                        locals,
                        mutable,
                        (kinds, loop_control, finalizers),
                        context,
                        output,
                    );
                }
                return Some(());
            }
            let finalizer = statement.finalizer.as_ref()?;
            encode_loop_effects(
                &Stmt::Block(statement.block.clone()),
                parameters,
                locals,
                mutable,
                (kinds, loop_control, &nested_finalizers),
                context,
                output,
            )?;
            return encode_loop_effects(
                &Stmt::Block(finalizer.clone()),
                parameters,
                locals,
                mutable,
                (kinds, loop_control, finalizers),
                context,
                output,
            );
        }
        if let Stmt::Throw(thrown) = statement {
            let mut encoded = Vec::new();
            encode_expression(
                thrown.arg.as_ref(),
                parameters,
                locals,
                context,
                &mut encoded,
            )?;
            let caught_kind = caught_throw_kind(&encoded)?;
            let kind = caught_kind.kind;
            let may_error = jit_tokens_may_error(&encoded);
            let start = output.len();
            output.extend(encoded);
            if loop_control.catch_active && may_error {
                insert_jit_error_checks(output, start);
            }
            let mut throw_kinds = kinds.clone();
            throw_kinds.insert(format!("\0throw-{}", throw_kinds.len()), kind);
            encode_finalizers(
                finalizers,
                loop_control.throw_finalizer_depth,
                (parameters, locals, mutable, &throw_kinds),
                loop_control,
                context,
                output,
            )?;
            output.push(
                if loop_control.catch_active {
                    if loop_control.catch_tagged {
                        output.push(format!("throwtag{}", caught_throw_tag(&caught_kind)?));
                        return Some(());
                    }
                    "throw"
                } else {
                    match caught_kind.prefix {
                        "rnl" => "throwoutrn",
                        "rbl" => "throwoutrb",
                        "rsl" => "throwoutrs",
                        "dnl" | "dbl" | "dsl" => "throwoutd",
                        _ => match kind {
                        JitKind::Number => "throwoutn",
                        JitKind::Boolean => "throwoutb",
                        JitKind::String => "throwouts",
                        JitKind::Dynamic => return None,
                        JitKind::Array | JitKind::Dictionary => return None,
                        },
                    }
                }
                .into(),
            );
            return Some(());
        }
        if let Stmt::Return(returned) = statement {
            let mut encoded = Vec::new();
            encode_expression(
                returned.arg.as_deref()?, parameters, locals, context, &mut encoded,
            )?;
            let kind = jit_expression_kind(&encoded).map(|(kind, _)| kind);
            if kind == Some(JitKind::Array) {
                encoded.push("arrayvalue".into());
            }
            let may_error = jit_tokens_may_error(&encoded);
            let start = output.len();
            output.extend(encoded);
            if loop_control.catch_active && may_error {
                insert_jit_error_checks(output, start);
            }
            let mut return_kinds = kinds.clone();
            return_kinds.insert(
                format!("\0return-{}", return_kinds.len()),
                kind.unwrap_or(JitKind::Number),
            );
            encode_finalizers(
                finalizers,
                0,
                (parameters, locals, mutable, &return_kinds),
                loop_control,
                context,
                output,
            )?;
            output.push("return".into());
            return Some(());
        }
        if let Stmt::While(loop_statement) = statement {
            output.extend(["loop".into(), "looptail".into()]);
            let start = output.len();
            encode_condition(loop_statement.test.as_ref(), parameters, locals, context, output)?;
            if loop_control.catch_active {
                insert_jit_error_checks(output, start);
            }
            output.push("while".into());
            context.loop_depth += 1;
            let body_result = encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                locals,
                mutable,
                (
                    kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            );
            context.loop_depth -= 1;
            body_result?;
            output.push("loopend".into());
            return Some(());
        }
        if let Stmt::DoWhile(loop_statement) = statement {
            output.push("loop".into());
            context.loop_depth += 1;
            let body_result = encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                locals,
                mutable,
                (
                    kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            );
            context.loop_depth -= 1;
            body_result?;
            output.push("looptail".into());
            let start = output.len();
            encode_condition(loop_statement.test.as_ref(), parameters, locals, context, output)?;
            if loop_control.catch_active {
                insert_jit_error_checks(output, start);
            }
            output.extend(["while".into(), "loopend".into()]);
            return Some(());
        }
        if let Stmt::For(loop_statement) = statement {
            let mut nested_locals = locals.clone();
            let mut nested_mutable = mutable.clone();
            let mut nested_kinds = kinds.clone();
            match loop_statement.init.as_ref() {
                Some(VarDeclOrExpr::Expr(initializer)) => encode_loop_expression(
                    initializer.as_ref(),
                    parameters,
                    &nested_locals,
                    &nested_mutable,
                    &nested_kinds,
                    context,
                    output,
                )?,
                None => {}
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
                            &mut nested_locals,
                            &mut nested_mutable,
                            &mut nested_kinds,
                            context,
                            output,
                        )?;
                    }
                }
            }
            output.push("loop".into());
            if let Some(test) = loop_statement.test.as_deref() {
                let start = output.len();
                encode_condition(test, parameters, &nested_locals, context, output)?;
                if loop_control.catch_active {
                    insert_jit_error_checks(output, start);
                }
            } else {
                output.extend([
                    format!("c{:016x}", 1.0f64.to_bits()),
                    "asbool".into(),
                ]);
            }
            output.push("while".into());
            context.loop_depth += 1;
            let body_result = encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                &nested_locals,
                &nested_mutable,
                (
                    &nested_kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            );
            context.loop_depth -= 1;
            body_result?;
            output.push("looptail".into());
            if let Some(update) = loop_statement.update.as_deref() {
                let start = output.len();
                encode_loop_expression(
                    update,
                    parameters,
                    &nested_locals,
                    &nested_mutable,
                    &nested_kinds,
                    context,
                    output,
                )?;
                if loop_control.catch_active {
                    insert_jit_error_checks(output, start);
                }
            }
            output.push("loopend".into());
            output.extend(std::iter::repeat_n(
                "drop".into(),
                nested_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        if let Stmt::ForOf(loop_statement) = statement {
            if loop_statement.is_await {
                return None;
            }
            let mut nested_locals = locals.clone();
            let mut nested_mutable = mutable.clone();
            let mut nested_kinds = kinds.clone();
            let mut source = Vec::new();
            encode_expression(
                loop_statement.right.as_ref(),
                parameters,
                &nested_locals,
                context,
                &mut source,
            )?;
            if source.len() == 1 {
                if let Some(untag) = jit_typed_array_union_untag(&source[0]) {
                    source.push(untag.into());
                }
            }
            match jit_expression_kind(&source)?.0 {
                JitKind::String => source.push("strarray".into()),
                JitKind::Array => {}
                _ => return None,
            }
            let array = array_prefix(&source)?;
            let element_kind = match array {
                "rn" => JitKind::Number,
                "rb" => JitKind::Boolean,
                "rs" => JitKind::String,
                _ => return None,
            };
            let source_index = nested_kinds.len();
            output.extend(source);
            nested_kinds.insert(format!("\0forof-source-{source_index}"), JitKind::Array);
            let source_local = format!("{array}l{source_index}");
            let index = nested_kinds.len();
            output.push(format!("c{:016x}", 0.0f64.to_bits()));
            nested_kinds.insert(format!("\0forof-index-{index}"), JitKind::Number);
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
                        || nested_locals.contains_key(name.id.sym.as_ref())
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
                        _ => return None,
                    }
                    let element_index = nested_kinds.len();
                    let prefix = match element_kind {
                        JitKind::Number => "ln",
                        JitKind::Boolean => "lb",
                        JitKind::String => "ls",
                        _ => return None,
                    };
                    nested_locals.insert(
                        name.id.sym.to_string(),
                        vec![format!("{prefix}{element_index}")],
                    );
                    nested_kinds.insert(name.id.sym.to_string(), element_kind);
                    if declaration.kind != VarDeclKind::Const {
                        nested_mutable.insert(name.id.sym.to_string());
                    }
                    element_index
                }
                ForHead::Pat(pattern) => {
                    let Pat::Ident(name) = pattern.as_ref() else {
                        return None;
                    };
                    if !nested_mutable.contains(name.id.sym.as_ref())
                        || nested_kinds.get(name.id.sym.as_ref())? != &element_kind
                    {
                        return None;
                    }
                    nested_locals
                        .get(name.id.sym.as_ref())?
                        .first()?
                        .get(2..)?
                        .parse::<usize>()
                        .ok()?
                }
                ForHead::UsingDecl(_) => return None,
            };
            output.push("loop".into());
            output.extend([
                index_local.clone(),
                source_local.clone(),
                "arraylen".into(),
                "<".into(),
                "while".into(),
                source_local,
                index_local.clone(),
                format!("{array}get"),
                format!("setl{element_index}"),
            ]);
            context.loop_depth += 1;
            let body_result = encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                &nested_locals,
                &nested_mutable,
                (
                    &nested_kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            );
            context.loop_depth -= 1;
            body_result?;
            output.extend([
                "looptail".into(),
                index_local,
                format!("c{:016x}", 1.0f64.to_bits()),
                "+".into(),
                format!("setl{index}"),
                "loopend".into(),
            ]);
            output.extend(std::iter::repeat_n(
                "drop".into(),
                nested_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        if let Stmt::ForIn(loop_statement) = statement {
            let mut nested_locals = locals.clone();
            let mut nested_mutable = mutable.clone();
            let mut nested_kinds = kinds.clone();
            context.loop_depth += 1;
            let body_result = encode_for_in_loop(
                loop_statement,
                parameters,
                &mut nested_locals,
                &mut nested_mutable,
                (&mut nested_kinds, loop_control, finalizers),
                context,
                output,
            );
            context.loop_depth -= 1;
            body_result?;
            output.extend(std::iter::repeat_n(
                "drop".into(),
                nested_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        let Stmt::Expr(statement) = statement else {
            return None;
        };
        let start = output.len();
        encode_loop_expression(
            statement.expr.as_ref(), parameters, locals, mutable, kinds, context, output,
        )?;
        if loop_control.catch_active {
            insert_jit_error_checks(output, start);
        }
        Some(())
    }

    fn encode_loop_expression(
        mut expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        kinds: &std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        while let Expr::Paren(parenthesized) = expression {
            expression = parenthesized.expr.as_ref();
        }
        if encode_callable_table_update(expression, parameters, locals, context, output).is_some() {
            return Some(());
        }
        match expression {
            Expr::Assign(assignment) => {
                if let AssignTarget::Pat(thaw_parser::ast::AssignTargetPat::Array(pattern)) =
                    &assignment.left
                {
                    if assignment.op != AssignOp::Assign {
                        return None;
                    }
                    let mut source = Vec::new();
                    encode_expression(
                        assignment.right.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut source,
                    )?;
                    if source.len() == 1 {
                        if let Some(untag) = jit_typed_array_union_untag(&source[0]) {
                            source.push(untag.into());
                        }
                    }
                    if jit_expression_kind(&source)?.0 != JitKind::Array {
                        return None;
                    }
                    let prefix = array_prefix(&source)?;
                    let element_kind = match prefix {
                        "rn" => JitKind::Number,
                        "rb" => JitKind::Boolean,
                        "rs" => JitKind::String,
                        _ => return None,
                    };
                    output.extend(source);
                    for (index, element) in pattern.elems.iter().enumerate() {
                        let Some(element) = element else {
                            continue;
                        };
                        if let Pat::Rest(rest) = element {
                            let Pat::Ident(name) = rest.arg.as_ref() else {
                                return None;
                            };
                            if index + 1 != pattern.elems.len()
                                || !mutable.contains(name.id.sym.as_ref())
                                || kinds.get(name.id.sym.as_ref())? != &JitKind::Array
                            {
                                return None;
                            }
                            let local = locals.get(name.id.sym.as_ref())?.first()?;
                            if local.get(..2)? != prefix {
                                return None;
                            }
                            output.extend([
                                "dup".into(),
                                format!("c{:016x}", (index as f64).to_bits()),
                                format!("c{:016x}", f64::INFINITY.to_bits()),
                                "arrayslice".into(),
                                "arrayhandle".into(),
                                format!("setl{}", loop_local_index(local)?),
                            ]);
                            continue;
                        }
                        let (name, default) = match element {
                            Pat::Ident(name) => (&name.id, None),
                            Pat::Assign(default) => {
                                let Pat::Ident(name) = default.left.as_ref() else {
                                    return None;
                                };
                                (&name.id, Some(default.right.as_ref()))
                            }
                            _ => return None,
                        };
                        if !mutable.contains(name.sym.as_ref()) {
                            return None;
                        }
                        let target_kind = *kinds.get(name.sym.as_ref())?;
                        if target_kind != element_kind {
                            return None;
                        }
                        let local = locals.get(name.sym.as_ref())?.first()?;
                        let mut value = vec![
                            "dup".into(),
                            format!("c{:016x}", (index as f64).to_bits()),
                            format!("{prefix}get"),
                        ];
                        if let Some(default) = default {
                            let mut fallback = Vec::new();
                            encode_expression(
                                default,
                                parameters,
                                locals,
                                context,
                                &mut fallback,
                            )?;
                            if target_kind == JitKind::Boolean {
                                let mut boolean = Vec::new();
                                append_boolean(fallback, &mut boolean)?;
                                fallback = boolean;
                            }
                            if jit_expression_kind(&fallback)?.0 != target_kind {
                                return None;
                            }
                            value.push("ifpresent".into());
                            value.push("else".into());
                            value.extend(fallback);
                            value.push("end".into());
                        }
                        output.extend(value);
                        output.push(format!("setl{}", loop_local_index(local)?));
                    }
                    output.push("drop".into());
                    return Some(());
                }
                if let AssignTarget::Pat(thaw_parser::ast::AssignTargetPat::Object(pattern)) =
                    &assignment.left
                {
                    if assignment.op != AssignOp::Assign {
                        return None;
                    }
                    let base = member_path(assignment.right.as_ref())?;
                    let mut bindings = Vec::new();
                    collect_fixed_object_pattern(pattern, "", &mut bindings)?;
                    if bindings.is_empty() {
                        return None;
                    }
                    for (path, name, default) in bindings {
                        if !mutable.contains(name.sym.as_ref()) {
                            return None;
                        }
                        let target_kind = *kinds.get(name.sym.as_ref())?;
                        let local = locals.get(name.sym.as_ref())?.first()?;
                        let path = format!("{base}{path}");
                        let mut value = locals
                            .get(&path)
                            .cloned()
                            .or_else(|| parameters.get(&path).map(|value| vec![value.clone()]))?;
                        let value_kind = jit_expression_kind(&value)?.0;
                        if value_kind != target_kind {
                            return None;
                        }
                        if let Some(default) = default {
                            let mut fallback = Vec::new();
                            encode_expression(
                                default,
                                parameters,
                                locals,
                                context,
                                &mut fallback,
                            )?;
                            if target_kind == JitKind::Boolean {
                                let mut boolean = Vec::new();
                                append_boolean(fallback, &mut boolean)?;
                                fallback = boolean;
                            }
                            if jit_expression_kind(&fallback)?.0 != target_kind {
                                return None;
                            }
                            value.push("ifpresent".into());
                            value.push("else".into());
                            value.extend(fallback);
                            value.push("end".into());
                        }
                        output.extend(value);
                        if target_kind == JitKind::Array {
                            if array_prefix(
                                parameters
                                    .get(&path)
                                    .map(std::slice::from_ref)
                                    .or_else(|| locals.get(&path).map(Vec::as_slice))?,
                            )? != local.get(..2)?
                            {
                                return None;
                            }
                            output.push("arrayhandle".into());
                        }
                        output.push(format!("setl{}", loop_local_index(local)?));
                    }
                    return Some(());
                }
                let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
                    encode_expression(expression, parameters, locals, context, output)?;
                    output.push("drop".into());
                    return Some(());
                };
                encode_local_assignment(
                    &name.id,
                    assignment.op,
                    assignment.right.as_ref(),
                    parameters,
                    locals,
                    mutable,
                    kinds,
                    context,
                    output,
                )?;
            }
            Expr::Update(update) => {
                let Expr::Ident(name) = update.arg.as_ref() else {
                    return None;
                };
                encode_local_update(name, update.op, locals, mutable, kinds, output)?;
            }
            expression => {
                encode_expression(expression, parameters, locals, context, output)?;
                output.push("drop".into());
            }
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_local_assignment(
        name: &Ident,
        operation: AssignOp,
        value_expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        kinds: &std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if !mutable.contains(name.sym.as_ref()) {
            return None;
        }
        let local = locals.get(name.sym.as_ref())?.first()?.clone();
        if let Some((local, helpers)) = locals
            .get(name.sym.as_ref())
            .and_then(|tokens| callable_local_alias(tokens))
        {
            if operation != AssignOp::Assign {
                return None;
            }
            return encode_callable_local_reassignment(
                value_expression,
                &local,
                &helpers,
                parameters,
                locals,
                context,
                output,
            );
        }
        let index = loop_local_index(&local)?;
        if operation != AssignOp::Assign {
            output.push(local.clone());
        }
        let mut value = Vec::new();
        encode_expression(value_expression, parameters, locals, context, &mut value)?;
        match kinds.get(name.sym.as_ref())? {
            JitKind::Boolean => value.push("asbool".into()),
            JitKind::Dynamic => {
                let value_kind = jit_expression_kind(&value)?.0;
                tag_jit_value(&mut value, value_kind)?;
            }
            JitKind::Array if array_prefix(&value)? != local.get(..2)? => return None,
            JitKind::Dictionary if dictionary_prefix(&value)? != local.get(..2)? => return None,
            _ => {}
        }
        output.extend(value);
        if kinds.get(name.sym.as_ref())? == &JitKind::Array && operation == AssignOp::Assign {
            output.push("arrayhandle".into());
        }
        if operation != AssignOp::Assign {
            output.push(
                match (kinds.get(name.sym.as_ref())?, operation) {
                    (JitKind::String, AssignOp::AddAssign) => "concat",
                    (JitKind::Number, AssignOp::AddAssign) => "+",
                    (JitKind::Number, AssignOp::SubAssign) => "-",
                    (JitKind::Number, AssignOp::MulAssign) => "*",
                    (JitKind::Number, AssignOp::DivAssign) => "/",
                    (JitKind::Number, AssignOp::ModAssign) => "%",
                    (JitKind::Number, AssignOp::LShiftAssign) => "shl",
                    (JitKind::Number, AssignOp::RShiftAssign) => "shr",
                    (JitKind::Number, AssignOp::ZeroFillRShiftAssign) => "ushr",
                    (JitKind::Number, AssignOp::BitOrAssign) => "bor",
                    (JitKind::Number, AssignOp::BitXorAssign) => "bxor",
                    (JitKind::Number, AssignOp::BitAndAssign) => "band",
                    (JitKind::Number, AssignOp::ExpAssign) => "pow",
                    _ => return None,
                }
                .into(),
            );
        }
        output.push(format!("setl{index}"));
        Some(())
    }

    fn encode_local_update(
        name: &Ident,
        operation: UpdateOp,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        kinds: &std::collections::HashMap<String, JitKind>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if !mutable.contains(name.sym.as_ref())
            || kinds.get(name.sym.as_ref())? != &JitKind::Number
        {
            return None;
        }
        let local = locals.get(name.sym.as_ref())?.first()?.clone();
        let index = loop_local_index(&local)?;
        output.extend([
            local,
            format!("c{:016x}", 1.0f64.to_bits()),
            match operation {
                UpdateOp::PlusPlus => "+".into(),
                UpdateOp::MinusMinus => "-".into(),
            },
            format!("setl{index}"),
        ]);
        Some(())
    }

    fn loop_local_index(token: &str) -> Option<usize> {
        token.get(token.find(|character: char| character.is_ascii_digit())?..)?
            .parse()
            .ok()
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_loop_declaration(
        step: LocalStep<'_>,
        forced_kind: Option<(JitKind, std::collections::HashSet<JitKind>)>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &mut std::collections::HashMap<String, Vec<String>>,
        mutable: &mut std::collections::HashSet<String>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let LocalStep::Declare {
            name,
            initializer,
            mutable: is_mutable,
        } = step
        else {
            return None;
        };
        if parameters.contains_key(name.sym.as_ref()) || locals.contains_key(name.sym.as_ref()) {
            return None;
        }
        let mut encoded = Vec::new();
        encode_expression(initializer, parameters, locals, context, &mut encoded)?;
        let value_kind = if boolean_literal(initializer) {
            JitKind::Boolean
        } else {
            jit_expression_kind(&encoded)?.0
        };
        let (kind, candidates) = forced_kind.unwrap_or_else(|| {
            let candidates = if value_kind == JitKind::Dynamic {
                encoded
                    .iter()
                    .filter_map(|token| match token.as_str() {
                        "tagnum" => Some(JitKind::Number),
                        "tagbool" => Some(JitKind::Boolean),
                        "tagstr" => Some(JitKind::String),
                        "tagrn" | "tagrb" | "tagrs" => Some(JitKind::Array),
                        "tagdn" | "tagdb" | "tagds" => Some(JitKind::Dictionary),
                        "tagtuple" => Some(JitKind::Array),
                        _ => None,
                    })
                    .collect()
            } else {
                std::iter::once(value_kind).collect()
            };
            (value_kind, candidates)
        });
        if kind == JitKind::Dynamic && value_kind != JitKind::Dynamic {
            tag_jit_value(&mut encoded, value_kind)?;
        } else if kind != value_kind {
            return None;
        }
        let prefix = match kind {
            JitKind::Number => "ln",
            JitKind::Boolean => "lb",
            JitKind::String => "ls",
            JitKind::Dynamic => "ld",
            JitKind::Array => match array_prefix(&encoded)? {
                "rn" => "rnl",
                "rb" => "rbl",
                "rs" => "rsl",
                _ => return None,
            },
            JitKind::Dictionary => match dictionary_prefix(&encoded)? {
                "dn" => "dnl",
                "db" => "dbl",
                "ds" => "dsl",
                _ => return None,
            },
        };
        let index = kinds.len();
        output.extend(encoded);
        if kind == JitKind::Boolean {
            output.push("asbool".into());
        } else if kind == JitKind::Array {
            output.push("arrayhandle".into());
        }
        let mut local = vec![format!("{prefix}{index}")];
        if kind == JitKind::Dynamic {
            for (candidate, marker) in [
                (JitKind::Number, "notnum"),
                (JitKind::Boolean, "notbool"),
                (JitKind::String, "notstr"),
            ] {
                if !candidates.contains(&candidate) {
                    local.push(marker.into());
                }
            }
        }
        locals.insert(name.sym.to_string(), local);
        kinds.insert(name.sym.to_string(), kind);
        if is_mutable {
            mutable.insert(name.sym.to_string());
        }
        Some(())
    }

    };
}

