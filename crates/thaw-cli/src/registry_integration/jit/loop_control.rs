macro_rules! jit_loop_control {
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

    };
}

