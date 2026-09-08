macro_rules! jit_loops {
    () => {
    fn encode_for_in_loop(
        statement: &thaw_parser::ast::ForInStmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &mut std::collections::HashMap<String, Vec<String>>,
        mutable: &mut std::collections::HashSet<String>,
        state: (
            &mut std::collections::HashMap<String, JitKind>,
            LoopControl,
            &[&thaw_parser::ast::BlockStmt],
        ),
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (kinds, loop_control, finalizers) = state;
        let mut source = Vec::new();
        encode_expression(
            statement.right.as_ref(),
            parameters,
            locals,
            context,
            &mut source,
        )?;
        if jit_expression_kind(&source)?.0 != JitKind::Dictionary {
            return None;
        }
        let source_prefix = dictionary_prefix(&source)?;
        output.extend(source);
        let source_index = kinds.len();
        kinds.insert(
            format!("\0forin-source-{source_index}"),
            JitKind::Dictionary,
        );
        let source_local = format!("{source_prefix}l{source_index}");

        let index = kinds.len();
        output.push(format!("c{:016x}", 0.0f64.to_bits()));
        kinds.insert(format!("\0forin-index-{index}"), JitKind::Number);
        let index_local = format!("ln{index}");

        let element_index = match &statement.left {
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
                if declaration.kind == VarDeclKind::Const {
                    locals.insert(
                        name.id.sym.to_string(),
                        vec![source_local.clone(), index_local.clone(), "dkeyat".into()],
                    );
                    None
                } else {
                    encode_string("", output)?;
                    let element_index = kinds.len();
                    locals.insert(
                        name.id.sym.to_string(),
                        vec![format!("ls{element_index}")],
                    );
                    kinds.insert(name.id.sym.to_string(), JitKind::String);
                    mutable.insert(name.id.sym.to_string());
                    Some(element_index)
                }
            }
            ForHead::Pat(pattern) => {
                let Pat::Ident(name) = pattern.as_ref() else {
                    return None;
                };
                if !mutable.contains(name.id.sym.as_ref())
                    || kinds.get(name.id.sym.as_ref())? != &JitKind::String
                {
                    return None;
                }
                Some(loop_local_index(
                    locals.get(name.id.sym.as_ref())?.first()?,
                )?)
            }
            ForHead::UsingDecl(_) => return None,
        };

        output.extend([
            "loop".into(),
            index_local.clone(),
            source_local.clone(),
            "dlen".into(),
            "<".into(),
            "while".into(),
        ]);
        if let Some(element_index) = element_index {
            output.extend([
                source_local,
                index_local.clone(),
                "dkeyat".into(),
                format!("setl{element_index}"),
            ]);
        }
        encode_loop_effects(
            statement.body.as_ref(),
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
        )?;
        output.extend([
            "looptail".into(),
            index_local,
            format!("c{:016x}", 1.0f64.to_bits()),
            "+".into(),
            format!("setl{index}"),
            "loopend".into(),
        ]);
        Some(())
    }


    fn encode_loop_switch(
        statement: &thaw_parser::ast::SwitchStmt,
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
        let mut discriminant = Vec::new();
        encode_expression(
            statement.discriminant.as_ref(),
            parameters,
            locals,
            context,
            &mut discriminant,
        )?;
        let kind = jit_expression_kind(&discriminant)?.0;
        if matches!(kind, JitKind::Array | JitKind::Dictionary) {
            return None;
        }
        output.extend(discriminant);
        output.push("switch".into());
        for case in &statement.cases {
            if let Some(test) = case.test.as_deref() {
                output.extend(["case".into(), "dup".into()]);
                let mut encoded = Vec::new();
                encode_expression(test, parameters, locals, context, &mut encoded)?;
                let test_kind = if boolean_literal(test) {
                    encoded.push("asbool".into());
                    JitKind::Boolean
                } else {
                    jit_expression_kind(&encoded)?.0
                };
                output.extend(encoded);
                if kind == JitKind::String && test_kind == JitKind::String {
                    output.extend([
                        "strcmp".into(),
                        format!("c{:016x}", 0.0f64.to_bits()),
                        "==".into(),
                    ]);
                } else if kind == test_kind
                    && matches!(kind, JitKind::Number | JitKind::Boolean)
                {
                    output.push("==".into());
                } else {
                    output.push("strictfalse".into());
                }
                output.push("casebody".into());
            } else {
                output.push("default".into());
            }
            for statement in &case.cons {
                encode_loop_effects(
                    statement,
                    parameters,
                    locals,
                    mutable,
                    (
                        kinds,
                        LoopControl {
                            break_target: BreakTarget::Switch,
                            break_finalizer_depth: finalizers.len(),
                            continue_finalizer_depth: loop_control.continue_finalizer_depth,
                            throw_finalizer_depth: loop_control.throw_finalizer_depth,
                            catch_active: loop_control.catch_active,
                            catch_tagged: loop_control.catch_tagged,
                        },
                        finalizers,
                    ),
                    context,
                    output,
                )?;
            }
        }
        output.push("switchend".into());
        Some(())
    }

    #[derive(Clone, PartialEq, Eq)]
    struct CaughtThrowKind {
        kind: JitKind,
        prefix: &'static str,
    }

    #[derive(Clone)]
    enum CatchProbe {
        Tag,
        ArrayIndex(u32),
        DictionaryKey(String),
    }

    fn caught_throw_kind(encoded: &[String]) -> Option<CaughtThrowKind> {
        let kind = jit_expression_kind(encoded)?.0;
        let prefix = match kind {
            JitKind::Number => "ln",
            JitKind::Boolean => "lb",
            JitKind::String => "ls",
            JitKind::Dynamic => "ld",
            JitKind::Array => match array_prefix(encoded)? {
                "rn" => "rnl",
                "rb" => "rbl",
                "rs" => "rsl",
                _ => return None,
            },
            JitKind::Dictionary => match dictionary_prefix(encoded)? {
                "dn" => "dnl",
                "db" => "dbl",
                "ds" => "dsl",
                _ => return None,
            },
        };
        Some(CaughtThrowKind { kind, prefix })
    }

    fn caught_throw_tag(kind: &CaughtThrowKind) -> Option<u8> {
        match kind.prefix {
            "ln" => Some(0),
            "lb" => Some(1),
            "ls" => Some(2),
            "rnl" => Some(3),
            "rbl" => Some(4),
            "rsl" => Some(5),
            "dnl" => Some(6),
            "dbl" => Some(7),
            "dsl" => Some(8),
            _ => None,
        }
    }

    fn collect_caught_native_error(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kinds: &mut Vec<CaughtThrowKind>,
    ) -> Option<()> {
        let mut encoded = Vec::new();
        if encode_expression(expression, parameters, locals, context, &mut encoded).is_none() {
            return Some(());
        }
        let error = CaughtThrowKind {
            kind: JitKind::String,
            prefix: "ls",
        };
        if jit_tokens_may_error(&encoded) && !kinds.contains(&error) {
            kinds.push(error);
        }
        Some(())
    }

    fn collect_caught_throw_kind(
        statement: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kinds: &mut Vec<CaughtThrowKind>,
    ) -> Option<()> {
        match statement {
            Stmt::Throw(thrown) => {
                let mut encoded = Vec::new();
                encode_expression(
                    thrown.arg.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                let error = CaughtThrowKind {
                    kind: JitKind::String,
                    prefix: "ls",
                };
                if jit_tokens_may_error(&encoded) && !kinds.contains(&error) {
                    kinds.push(error);
                }
                let thrown_kind = caught_throw_kind(&encoded)?;
                if !kinds.contains(&thrown_kind) {
                    kinds.push(thrown_kind);
                }
            }
            Stmt::Expr(statement) => collect_caught_native_error(
                statement.expr.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::Return(statement) => {
                if let Some(expression) = statement.arg.as_deref() {
                    collect_caught_native_error(
                        expression, parameters, locals, context, kinds,
                    )?;
                }
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                }
            }
            Stmt::If(branch) => {
                collect_caught_native_error(
                    branch.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                collect_caught_throw_kind(
                    branch.cons.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                if let Some(alternate) = branch.alt.as_deref() {
                    collect_caught_throw_kind(
                        alternate,
                        parameters,
                        locals,
                        context,
                        kinds,
                    )?;
                }
            }
            Stmt::While(statement) => {
                collect_caught_native_error(
                    statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                collect_caught_throw_kind(
                    statement.body.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
            }
            Stmt::DoWhile(statement) => {
                collect_caught_throw_kind(
                    statement.body.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                collect_caught_native_error(
                    statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
            }
            Stmt::For(statement) => collect_caught_throw_kind(
                statement.body.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::ForIn(statement) => collect_caught_throw_kind(
                statement.body.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::ForOf(statement) => collect_caught_throw_kind(
                statement.body.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::Switch(statement) => {
                for statement in statement.cases.iter().flat_map(|case| &case.cons) {
                    collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                }
            }
            Stmt::Try(statement) => {
                if let Some(handler) = &statement.handler {
                    let mut nested_kinds = Vec::new();
                    for statement in &statement.block.stmts {
                        collect_caught_throw_kind(
                            statement,
                            parameters,
                            locals,
                            context,
                            &mut nested_kinds,
                        )?;
                    }
                    let mut handler_locals = locals.clone();
                    if let (Some(Pat::Ident(identifier)), [nested_kind]) =
                        (&handler.param, nested_kinds.as_slice())
                    {
                        handler_locals.insert(
                            identifier.id.sym.to_string(),
                            vec![format!("{}0", nested_kind.prefix)],
                        );
                    }
                    for statement in &handler.body.stmts {
                        collect_caught_throw_kind(
                            statement,
                            parameters,
                            &handler_locals,
                            context,
                            kinds,
                        )?;
                    }
                } else {
                    for statement in &statement.block.stmts {
                        collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                    }
                }
            }
            _ => {}
        }
        Some(())
    }

    fn dynamic_catch_narrowing(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(
        String,
        usize,
        usize,
        CaughtThrowKind,
        Option<CaughtThrowKind>,
        CatchProbe,
    )> {
        let typeof_target = |expression: &Expr| {
            let Expr::Unary(unary) = expression else {
                return None;
            };
            if unary.op != UnaryOp::TypeOf {
                return None;
            }
            match unary.arg.as_ref() {
                Expr::Ident(identifier) => Some((identifier.sym.to_string(), CatchProbe::Tag)),
                Expr::Member(member) => {
                    let Expr::Ident(identifier) = member.obj.as_ref() else {
                        return None;
                    };
                    let probe = match &member.prop {
                        MemberProp::Ident(property) => {
                            CatchProbe::DictionaryKey(property.sym.to_string())
                        }
                        MemberProp::Computed(property) => match property.expr.as_ref() {
                            Expr::Lit(Lit::Num(index))
                                if index.value >= 0.0
                                    && index.value.fract() == 0.0
                                    && index.value <= u32::MAX as f64 =>
                            {
                                CatchProbe::ArrayIndex(index.value as u32)
                            }
                            Expr::Lit(Lit::Str(key)) => CatchProbe::DictionaryKey(
                                key.value.to_string_lossy().into_owned(),
                            ),
                            _ => return None,
                        },
                        _ => return None,
                    };
                    Some((identifier.sym.to_string(), probe))
                }
                _ => None,
            }
        };
        let literal = |expression: &Expr| {
            let Expr::Lit(Lit::Str(value)) = expression else {
                return None;
            };
            Some(value.value.to_string_lossy().into_owned())
        };
        let (name, selector, probe) = if let Expr::Bin(binary) = expression {
            if !matches!(binary.op, BinaryOp::EqEq | BinaryOp::EqEqEq) {
                return None;
            }
            let ((name, probe), type_name) = typeof_target(binary.left.as_ref())
                .zip(literal(binary.right.as_ref()))
                .or_else(|| {
                    literal(binary.left.as_ref())
                        .zip(typeof_target(binary.right.as_ref()))
                        .map(|(literal, target)| (target, literal))
                })?;
            let base = match &probe {
                CatchProbe::Tag => 0,
                CatchProbe::ArrayIndex(_) => 3,
                CatchProbe::DictionaryKey(_) => 6,
            };
            let selector = match type_name.as_str() {
                "number" => Some(base),
                "boolean" => Some(base + 1),
                "string" => Some(base + 2),
                "object" => return None,
                _ => return None,
            };
            (name, selector, probe)
        } else {
            let Expr::Call(call) = expression else {
                return None;
            };
            let Callee::Expr(callee) = &call.callee else {
                return None;
            };
            let Expr::Member(member) = callee.as_ref() else {
                return None;
            };
            let (Expr::Ident(object), MemberProp::Ident(property)) =
                (member.obj.as_ref(), &member.prop)
            else {
                return None;
            };
            if object.sym != *"Array" || property.sym != *"isArray" {
                return None;
            }
            let [argument] = call.args.as_slice() else {
                return None;
            };
            if argument.spread.is_some() {
                return None;
            }
            let Expr::Ident(identifier) = argument.expr.as_ref() else {
                return None;
            };
            (identifier.sym.to_string(), None, CatchProbe::Tag)
        };
        let [marker] = locals.get(&name)?.as_slice() else {
            return None;
        };
        let marker = marker.strip_prefix('x')?;
        let mut fields = marker.splitn(3, ':');
        let tag_index = fields.next()?.parse().ok()?;
        let value_index = fields.next()?.parse().ok()?;
        let variants = fields
            .next()?
            .split('|')
            .map(|variant| {
                let (tag, prefix) = variant.split_once('=')?;
                let tag = tag.parse::<u8>().ok()?;
                let (kind, prefix) = match prefix {
                    "ln" => (JitKind::Number, "ln"),
                    "lb" => (JitKind::Boolean, "lb"),
                    "ls" => (JitKind::String, "ls"),
                    "rnl" => (JitKind::Array, "rnl"),
                    "rbl" => (JitKind::Array, "rbl"),
                    "rsl" => (JitKind::Array, "rsl"),
                    "dnl" => (JitKind::Dictionary, "dnl"),
                    "dbl" => (JitKind::Dictionary, "dbl"),
                    "dsl" => (JitKind::Dictionary, "dsl"),
                    _ => return None,
                };
                Some((tag, CaughtThrowKind { kind, prefix }))
            })
            .collect::<Option<Vec<_>>>()?;
        let selected = if let Some(expected_tag) = selector {
            variants
                .iter()
                .find(|(tag, _)| *tag == expected_tag)?
                .1
                .clone()
        } else {
            let mut objects = variants.iter().filter(|(_, kind)| kind.kind == JitKind::Array);
            let selected = objects.next()?.1.clone();
            if objects.next().is_some() {
                return None;
            }
            selected
        };
        let selected_tag = caught_throw_tag(&selected)?;
        let alternate = (matches!(&probe, CatchProbe::Tag) && variants.len() == 2)
            .then(|| {
                variants
                    .iter()
                    .find(|(tag, _)| *tag != selected_tag)
                    .map(|(_, kind)| kind.clone())
            })
            .flatten();
        Some((name, tag_index, value_index, selected, alternate, probe))
    }

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

    fn encode_steps_and_body(
        steps: Vec<LocalStep<'_>>,
        body: NumericBody<'_>,
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let NumericBody::Statements(statements) = &body {
            if matches!(statements.first(), Some(Stmt::While(_))) {
                return encode_while_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::For(_))) {
                return encode_for_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::DoWhile(_))) {
                return encode_do_while_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::ForOf(_))) {
                return encode_for_of_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::ForIn(_))) {
                return encode_for_in_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
        }
        let mut mutable = std::collections::HashSet::new();
        let mut runtime_locals = 0usize;
        let mut runtime_kinds = std::collections::HashMap::new();
        let materialize_control_locals = matches!(
            &body,
            NumericBody::Statements(
                [Stmt::Block(_)
                    | Stmt::If(_)
                    | Stmt::Switch(_)
                    | Stmt::Try(_)
                    | Stmt::Labeled(_), ..]
                )
        );
        let mut control_callable_candidates = std::collections::HashMap::new();
        let mut control_value_assignments = std::collections::HashMap::new();
        if materialize_control_locals {
            let NumericBody::Statements(statements) = &body else {
                return None;
            };
            for statement in *statements {
                collect_loop_callable_assignments(
                    statement,
                    context,
                    &mut control_callable_candidates,
                );
                collect_local_value_assignments(statement, &mut control_value_assignments);
            }
        }
        for step in steps {
            match step {
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable: is_mutable,
                } => {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    if let Some(mut alias) = encode_callable_member_snapshot(
                        initializer,
                        parameters,
                        &locals,
                        context,
                        &mut runtime_locals,
                        output,
                    ) {
                        if materialize_control_locals {
                            let (local, source_helpers) =
                                callable_local_alias(std::slice::from_ref(&alias))?;
                            let mut helpers = source_helpers.clone();
                            for helper in control_callable_candidates
                                .remove(name.sym.as_ref())
                                .into_iter()
                                .flatten()
                            {
                                if !helpers.contains(&helper) {
                                    helpers.push(helper);
                                }
                            }
                            if helpers != source_helpers {
                                encode_callable_selection_remap(
                                    &source_helpers,
                                    &helpers,
                                    output,
                                )?;
                                alias = format!(
                                    "{CALLABLE_LOCAL_PREFIX}{local}:{}",
                                    helpers.join("|")
                                );
                            }
                        }
                        locals.insert(name.sym.to_string(), vec![alias]);
                        runtime_kinds.insert(name.sym.to_string(), JitKind::Number);
                        if runtime_kinds.len() != runtime_locals {
                            return None;
                        }
                        if is_mutable {
                            mutable.insert(name.sym.to_string());
                        }
                        continue;
                    }
                    if let Some(alias) = encode_callable_member_alias(
                        initializer,
                        parameters,
                        &locals,
                        context,
                    ) {
                        locals.insert(name.sym.to_string(), vec![alias]);
                        if is_mutable {
                            mutable.insert(name.sym.to_string());
                        }
                        continue;
                    }
                    if materialize_control_locals {
                        if let Some(mut helpers) = finite_callable_names(initializer).filter(
                            |helpers| {
                                helpers
                                    .iter()
                                    .all(|helper| context.helpers.contains_key(helper))
                            },
                        ) {
                            for helper in control_callable_candidates
                                .remove(name.sym.as_ref())
                                .into_iter()
                                .flatten()
                            {
                                if !helpers.contains(&helper) {
                                    helpers.push(helper);
                                }
                            }
                            if helpers.len() > 1 {
                                encode_callable_assignment_selection(
                                    initializer,
                                    &helpers,
                                    parameters,
                                    &locals,
                                    context,
                                    output,
                                )?;
                                let index = runtime_kinds.len();
                                locals.insert(
                                    name.sym.to_string(),
                                    vec![format!(
                                        "{CALLABLE_LOCAL_PREFIX}ln{index}:{}",
                                        helpers.join("|")
                                    )],
                                );
                                runtime_kinds.insert(name.sym.to_string(), JitKind::Number);
                                runtime_locals = runtime_kinds.len();
                                if is_mutable {
                                    mutable.insert(name.sym.to_string());
                                }
                                continue;
                            }
                        }
                    }
                    if let Some(alias) = callable_alias(initializer, &locals, context) {
                        locals.insert(
                            name.sym.to_string(),
                            vec![format!("{CALLABLE_ALIAS_PREFIX}{alias}")],
                        );
                        if is_mutable {
                            mutable.insert(name.sym.to_string());
                        }
                        continue;
                    }
                    if materialize_control_locals && is_mutable {
                        let forced_kind = control_value_assignments
                            .remove(name.sym.as_ref())
                            .and_then(|assignments| {
                                widened_local_kind(
                                    name,
                                    initializer,
                                    &assignments,
                                    parameters,
                                    &locals,
                                    context,
                                )
                            });
                        encode_loop_declaration(
                            LocalStep::Declare {
                                name,
                                initializer,
                                mutable: true,
                            },
                            forced_kind,
                            parameters,
                            &mut locals,
                            &mut mutable,
                            &mut runtime_kinds,
                            context,
                            output,
                        )?;
                        runtime_locals = runtime_kinds.len();
                        continue;
                    }
                    let mut encoded = Vec::new();
                    encode_expression(initializer, parameters, &locals, context, &mut encoded)?;
                    if !stable_jit_tokens(&encoded) {
                        return None;
                    }
                    locals.insert(name.sym.to_string(), encoded);
                    if is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                }
                LocalStep::DestructureArray {
                    bindings,
                    rest,
                    initializer,
                    mutable: is_mutable,
                    assign_existing,
                } => {
                    let mut source = Vec::new();
                    encode_expression(initializer, parameters, &locals, context, &mut source)?;
                    if source.len() == 1 {
                        if let Some(untag) = jit_typed_array_union_untag(&source[0]) {
                            source.push(untag.into());
                        }
                    }
                    if jit_expression_kind(&source)?.0 != JitKind::Array {
                        return None;
                    }
                    let prefix = array_prefix(&source)?;
                    let local_prefix = match prefix {
                        "rn" => "rnl",
                        "rb" => "rbl",
                        "rs" => "rsl",
                        _ => return None,
                    };
                    let names = bindings
                        .iter()
                        .map(|(_, name, _)| *name)
                        .chain(rest.iter().map(|(_, name)| *name));
                    if names.clone().any(|name| {
                        if assign_existing {
                            !mutable.contains(name.sym.as_ref())
                        } else {
                            parameters.contains_key(name.sym.as_ref())
                                || locals.contains_key(name.sym.as_ref())
                        }
                    }) {
                        return None;
                    }
                    let source_index = runtime_kinds.len();
                    output.extend(source);
                    output.push("arrayhandle".into());
                    runtime_kinds.insert(
                        format!("\0destructure-source-{source_index}"),
                        JitKind::Array,
                    );
                    runtime_locals = runtime_kinds.len();
                    let source_local = format!("{local_prefix}{source_index}");
                    for (index, name, default) in bindings {
                        let mut value = vec![source_local.clone()];
                        value.push(format!("c{:016x}", (index as f64).to_bits()));
                        value.push(format!("{prefix}get"));
                        if let Some(default) = default {
                            let value_kind = jit_expression_kind(&value)?.0;
                            let mut fallback = Vec::new();
                            encode_expression(
                                default,
                                parameters,
                                &locals,
                                context,
                                &mut fallback,
                            )?;
                            if value_kind == JitKind::Boolean {
                                let mut boolean = Vec::new();
                                append_boolean(fallback, &mut boolean)?;
                                fallback = boolean;
                            }
                            if jit_expression_kind(&fallback)?.0 != value_kind {
                                return None;
                            }
                            value.push("ifpresent".into());
                            value.push("else".into());
                            value.extend(fallback);
                            value.push("end".into());
                        }
                        if assign_existing && runtime_kinds.contains_key(name.sym.as_ref()) {
                            let target_kind = *runtime_kinds.get(name.sym.as_ref())?;
                            if jit_expression_kind(&value)?.0 != target_kind {
                                return None;
                            }
                            let target =
                                loop_local_index(locals.get(name.sym.as_ref())?.first()?)?;
                            output.extend(value);
                            output.push(format!("setl{target}"));
                        } else {
                            locals.insert(name.sym.to_string(), value);
                        }
                        if is_mutable && !assign_existing {
                            mutable.insert(name.sym.to_string());
                        }
                    }
                    if let Some((index, name)) = rest {
                        output.push(source_local);
                        output.push(format!("c{:016x}", (index as f64).to_bits()));
                        output.push(format!("c{:016x}", f64::INFINITY.to_bits()));
                        output.push("arrayslice".into());
                        output.push("arrayhandle".into());
                        if assign_existing && runtime_kinds.contains_key(name.sym.as_ref()) {
                            if runtime_kinds.get(name.sym.as_ref())? != &JitKind::Array {
                                return None;
                            }
                            let local = locals.get(name.sym.as_ref())?.first()?;
                            if !local.starts_with(local_prefix) {
                                return None;
                            }
                            output.push(format!("setl{}", loop_local_index(local)?));
                        } else {
                            let rest_index = runtime_kinds.len();
                            locals.insert(
                                name.sym.to_string(),
                                vec![format!("{local_prefix}{rest_index}")],
                            );
                            runtime_kinds.insert(name.sym.to_string(), JitKind::Array);
                            runtime_locals = runtime_kinds.len();
                        }
                        if is_mutable && !assign_existing {
                            mutable.insert(name.sym.to_string());
                        }
                    }
                }
                LocalStep::DestructureObject {
                    bindings,
                    initializer,
                    mutable: is_mutable,
                    assign_existing,
                } => {
                    if bindings.iter().any(|(_, name, _)| {
                        if assign_existing {
                            !mutable.contains(name.sym.as_ref())
                        } else {
                            parameters.contains_key(name.sym.as_ref())
                                || locals.contains_key(name.sym.as_ref())
                        }
                    }) {
                        return None;
                    }
                    let base = member_path(initializer);
                    let mut materialized = std::collections::HashMap::new();
                    if base.is_none() {
                        let requested = bindings
                            .iter()
                            .map(|(path, _, _)| path.clone())
                            .collect();
                        if let Expr::Call(call) = initializer {
                            materialize_fixed_call(
                                call,
                                &requested,
                                parameters,
                                &locals,
                                context,
                                &mut runtime_kinds,
                                &mut materialized,
                                output,
                            )?;
                        } else {
                            materialize_fixed_literal(
                                initializer,
                                "",
                                &requested,
                                parameters,
                                &locals,
                                context,
                                &mut runtime_kinds,
                                &mut materialized,
                                output,
                            )?;
                        }
                        runtime_locals = runtime_kinds.len();
                    }
                    for (path, name, default) in bindings {
                        let path = base
                            .as_ref()
                            .map_or_else(|| path.clone(), |base| format!("{base}{path}"));
                        let mut value = materialized
                            .get(&path)
                            .cloned()
                            .or_else(|| locals.get(&path).cloned())
                            .or_else(|| parameters.get(&path).map(|value| vec![value.clone()]))?;
                        if let Some(default) = default {
                            let value_kind = jit_expression_kind(&value)?.0;
                            let mut fallback = Vec::new();
                            encode_expression(
                                default,
                                parameters,
                                &locals,
                                context,
                                &mut fallback,
                            )?;
                            if value_kind == JitKind::Boolean {
                                let mut boolean = Vec::new();
                                append_boolean(fallback, &mut boolean)?;
                                fallback = boolean;
                            }
                            if jit_expression_kind(&fallback)?.0 != value_kind {
                                return None;
                            }
                            value.push("ifpresent".into());
                            value.push("else".into());
                            value.extend(fallback);
                            value.push("end".into());
                        }
                        if assign_existing && runtime_kinds.contains_key(name.sym.as_ref()) {
                            let target_kind = *runtime_kinds.get(name.sym.as_ref())?;
                            if jit_expression_kind(&value)?.0 != target_kind {
                                return None;
                            }
                            let target =
                                loop_local_index(locals.get(name.sym.as_ref())?.first()?)?;
                            output.extend(value);
                            output.push(format!("setl{target}"));
                        } else {
                            locals.insert(name.sym.to_string(), value);
                        }
                        if is_mutable && !assign_existing {
                            mutable.insert(name.sym.to_string());
                        }
                    }
                }
                LocalStep::Assign {
                    name,
                    operation,
                    value,
                } => {
                    if let Some(slot) = context.module_globals.get(name.sym.as_ref()).copied() {
                        let mut encoded = vec![format!("c{:016x}", (slot as f64).to_bits())];
                        if operation != AssignOp::Assign {
                            encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        }
                        encode_expression(value, parameters, &locals, context, &mut encoded)?;
                        if operation != AssignOp::Assign {
                            encoded.push(
                                match operation {
                                    AssignOp::AddAssign => "+",
                                    AssignOp::SubAssign => "-",
                                    AssignOp::MulAssign => "*",
                                    AssignOp::DivAssign => "/",
                                    AssignOp::ModAssign => "%",
                                    AssignOp::LShiftAssign => "shl",
                                    AssignOp::RShiftAssign => "shr",
                                    AssignOp::ZeroFillRShiftAssign => "ushr",
                                    AssignOp::BitOrAssign => "bor",
                                    AssignOp::BitXorAssign => "bxor",
                                    AssignOp::BitAndAssign => "band",
                                    AssignOp::ExpAssign => "pow",
                                    _ => return None,
                                }
                                .into(),
                            );
                        }
                        encoded.extend(["globalset".into(), "drop".into()]);
                        output.extend(encoded);
                        continue;
                    }
                    if runtime_kinds.contains_key(name.sym.as_ref()) {
                        encode_local_assignment(
                            name,
                            operation,
                            value,
                            parameters,
                            &locals,
                            &mutable,
                            &runtime_kinds,
                            context,
                            output,
                        )?;
                        continue;
                    }
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    if operation == AssignOp::Assign {
                        if let Some((local, helpers)) = locals
                            .get(name.sym.as_ref())
                            .and_then(|tokens| callable_local_alias(tokens))
                        {
                            let mut selection = Vec::new();
                            let mut unused_local = 0usize;
                            if let Some(alias) = encode_callable_member_snapshot(
                                value,
                                parameters,
                                &locals,
                                context,
                                &mut unused_local,
                                &mut selection,
                            ) {
                                let (_, selected_helpers) =
                                    callable_local_alias(&[alias])?;
                                if helpers != selected_helpers {
                                    return None;
                                }
                                output.extend(selection);
                                output.push(format!("setl{}", loop_local_index(&local)?));
                                continue;
                            }
                        }
                        if let Some(alias) = encode_callable_member_alias(
                            value,
                            parameters,
                            &locals,
                            context,
                        ) {
                            locals.insert(name.sym.to_string(), vec![alias]);
                            continue;
                        }
                        if let Some(alias) = callable_alias(value, &locals, context) {
                            locals.insert(
                                name.sym.to_string(),
                                vec![format!("{CALLABLE_ALIAS_PREFIX}{alias}")],
                            );
                            continue;
                        }
                    }
                    let mut encoded = Vec::new();
                    if operation == AssignOp::AddAssign {
                        let mut right = Vec::new();
                        encode_expression(value, parameters, &locals, context, &mut right)?;
                        append_add(
                            locals.get(name.sym.as_ref())?.clone(),
                            right,
                            &mut encoded,
                        )?;
                    } else {
                        if operation != AssignOp::Assign {
                            encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        }
                        encode_expression(value, parameters, &locals, context, &mut encoded)?;
                    }
                    if operation != AssignOp::Assign && operation != AssignOp::AddAssign {
                        encoded.push(
                            match operation {
                                AssignOp::SubAssign => "-",
                                AssignOp::MulAssign => "*",
                                AssignOp::DivAssign => "/",
                                AssignOp::ModAssign => "%",
                                AssignOp::LShiftAssign => "shl",
                                AssignOp::RShiftAssign => "shr",
                                AssignOp::ZeroFillRShiftAssign => "ushr",
                                AssignOp::BitOrAssign => "bor",
                                AssignOp::BitXorAssign => "bxor",
                                AssignOp::BitAndAssign => "band",
                                AssignOp::ExpAssign => "pow",
                                _ => return None,
                            }
                            .into(),
                        );
                    }
                    if !stable_jit_tokens(&encoded) {
                        return None;
                    }
                    locals.insert(name.sym.to_string(), encoded);
                }
                LocalStep::Update { name, operation } => {
                    if let Some(slot) = context.module_globals.get(name.sym.as_ref()).copied() {
                        output.push(format!("c{:016x}", (slot as f64).to_bits()));
                        output.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        output.push(format!("c{:016x}", 1.0f64.to_bits()));
                        output.push(
                            match operation {
                                UpdateOp::PlusPlus => "+",
                                UpdateOp::MinusMinus => "-",
                            }
                            .into(),
                        );
                        output.extend(["globalset".into(), "drop".into()]);
                        continue;
                    }
                    if runtime_kinds.contains_key(name.sym.as_ref()) {
                        encode_local_update(
                            name,
                            operation,
                            &locals,
                            &mutable,
                            &runtime_kinds,
                            output,
                        )?;
                        continue;
                    }
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    let mut encoded = locals.get(name.sym.as_ref())?.clone();
                    encoded.push(format!("c{:016x}", 1.0f64.to_bits()));
                    encoded.push(
                        match operation {
                            UpdateOp::PlusPlus => "+",
                            UpdateOp::MinusMinus => "-",
                        }
                        .into(),
                    );
                    locals.insert(name.sym.to_string(), encoded);
                }
                LocalStep::Effect(expression) => {
                    let mut effect = Vec::new();
                    if encode_callable_table_update(
                        expression,
                        parameters,
                        &locals,
                        context,
                        &mut effect,
                    )
                    .is_some()
                    {
                        output.extend(effect);
                        continue;
                    }
                    effect.clear();
                    encode_expression(expression, parameters, &locals, context, &mut effect)?;
                    effect.push("drop".into());
                    output.extend(effect);
                }
            }
        }
        let mut tail = Vec::new();
        if let NumericBody::Statements(statements) = body {
            encode_callable_alias_flow(
                statements,
                parameters,
                &locals,
                &mutable,
                context,
                &mut tail,
            )?;
        } else {
            encode_numeric_body(body, parameters, &locals, context, &mut tail)?;
        }
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), runtime_locals));
        Some(())
    }

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
