macro_rules! jit_expressions {
    () => {
    fn encode_expression(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        struct OptionalChainOperation {
            receiver_presence: Option<Vec<String>>,
            receiver: Vec<String>,
            continuation: Vec<String>,
        }

        fn optional_tokens<'a>(
            expression: &Expr,
            parameters: &'a std::collections::HashMap<String, String>,
        ) -> Option<(Vec<String>, &'a str)> {
            let expression = match expression {
                Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
                expression => expression,
            };
            let Expr::Ident(identifier) = expression else {
                return None;
            };
            let (presence, value) = parameters
                .get(identifier.sym.as_ref())?
                .strip_prefix("optional:")?
                .split_once(':')?;
            Some((presence.split(',').map(str::to_owned).collect(), value))
        }

        fn flatten_nullish<'a>(expression: &'a Expr, output: &mut Vec<&'a Expr>) {
            if let Expr::Bin(binary) = expression {
                if binary.op == BinaryOp::NullishCoalescing {
                    flatten_nullish(binary.left.as_ref(), output);
                    flatten_nullish(binary.right.as_ref(), output);
                    return;
                }
            }
            output.push(expression);
        }

        fn optional_chain_operation(
            chain: &thaw_parser::ast::OptChainExpr,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<OptionalChainOperation> {
            const RECEIVER: &str = "__thaw_optional_receiver";

            fn ordinary_expression(expression: &Expr) -> Option<Expr> {
                match expression {
                    Expr::OptChain(chain) => ordinary_chain(chain),
                    Expr::Member(member) => {
                        let mut member = member.clone();
                        member.obj = Box::new(ordinary_expression(member.obj.as_ref())?);
                        Some(Expr::Member(member))
                    }
                    Expr::Call(call) => {
                        let mut call = call.clone();
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = ordinary_expression(callee.as_ref())?;
                        }
                        Some(Expr::Call(call))
                    }
                    Expr::Paren(parenthesized) => {
                        let mut parenthesized = parenthesized.clone();
                        parenthesized.expr =
                            Box::new(ordinary_expression(parenthesized.expr.as_ref())?);
                        Some(Expr::Paren(parenthesized))
                    }
                    expression => Some(expression.clone()),
                }
            }

            fn split_expression(
                expression: &Expr,
                receiver: &mut Option<Expr>,
            ) -> Option<Expr> {
                match expression {
                    Expr::OptChain(chain) => split_chain(chain, receiver),
                    Expr::Member(member) => {
                        let mut member = member.clone();
                        member.obj = Box::new(split_expression(member.obj.as_ref(), receiver)?);
                        Some(Expr::Member(member))
                    }
                    Expr::Call(call) => {
                        let mut call = call.clone();
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = split_expression(callee.as_ref(), receiver)?;
                        }
                        Some(Expr::Call(call))
                    }
                    Expr::Paren(parenthesized) => {
                        let mut parenthesized = parenthesized.clone();
                        parenthesized.expr =
                            Box::new(split_expression(parenthesized.expr.as_ref(), receiver)?);
                        Some(Expr::Paren(parenthesized))
                    }
                    expression => Some(expression.clone()),
                }
            }

            fn split_chain(
                chain: &thaw_parser::ast::OptChainExpr,
                receiver: &mut Option<Expr>,
            ) -> Option<Expr> {
                match chain.base.as_ref() {
                    OptChainBase::Member(member) => {
                        let mut member = member.clone();
                        if chain.optional && receiver.is_none() {
                            *receiver = Some(ordinary_expression(member.obj.as_ref())?);
                            member.obj = Box::new(Expr::Ident(RECEIVER.into()));
                        } else {
                            member.obj = Box::new(split_expression(member.obj.as_ref(), receiver)?);
                        }
                        Some(Expr::Member(member))
                    }
                    OptChainBase::Call(call) => {
                        if chain.optional {
                            return None;
                        }
                        let mut call = CallExpr::from(call.clone());
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = split_expression(callee.as_ref(), receiver)?;
                        }
                        Some(Expr::Call(call))
                    }
                }
            }

            fn ordinary_chain(chain: &thaw_parser::ast::OptChainExpr) -> Option<Expr> {
                match chain.base.as_ref() {
                    OptChainBase::Member(member) => {
                        let mut member = member.clone();
                        member.obj = Box::new(ordinary_expression(member.obj.as_ref())?);
                        Some(Expr::Member(member))
                    }
                    OptChainBase::Call(call) => {
                        let mut call = CallExpr::from(call.clone());
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = ordinary_expression(callee.as_ref())?;
                        }
                        Some(Expr::Call(call))
                    }
                }
            }

            fn optional_root<'a>(
                expression: &Expr,
                parameters: &'a std::collections::HashMap<String, String>,
            ) -> Option<(Vec<String>, &'a str, String)> {
                match expression {
                    Expr::Ident(identifier) => {
                        let (presence, value) = parameters
                            .get(identifier.sym.as_ref())?
                            .strip_prefix("optional:")?
                            .split_once(':')?;
                        Some((
                            presence.split(',').map(str::to_owned).collect(),
                            value,
                            identifier.sym.to_string(),
                        ))
                    }
                    Expr::Member(member) => optional_root(member.obj.as_ref(), parameters),
                    Expr::Call(call) => match &call.callee {
                        Callee::Expr(callee) => optional_root(callee.as_ref(), parameters),
                        _ => None,
                    },
                    Expr::Paren(parenthesized) => {
                        optional_root(parenthesized.expr.as_ref(), parameters)
                    }
                    _ => None,
                }
            }

            let ordinary = ordinary_chain(chain)?;
            if let Some((presence, value, receiver)) = optional_root(&ordinary, parameters) {
                let mut unwrapped = parameters.clone();
                unwrapped.insert(receiver, value.into());
                let mut operation = Vec::new();
                encode_expression(
                    &ordinary,
                    &unwrapped,
                    locals,
                    context,
                    &mut operation,
                )?;
                return Some(OptionalChainOperation {
                    receiver_presence: Some(presence),
                    receiver: operation,
                    continuation: Vec::new(),
                });
            }

            let mut source = None;
            let ordinary = split_chain(chain, &mut source)?;
            let source = source?;
            let mut receiver = Vec::new();
            encode_expression(
                &source,
                parameters,
                locals,
                context,
                &mut receiver,
            )?;
            if !jit_operation_may_be_absent(&receiver) {
                return None;
            }
            let receiver_token = match jit_expression_kind(&receiver)?.0 {
                JitKind::Number => format!("a{RECEIVER}"),
                JitKind::Boolean => format!("b{RECEIVER}"),
                JitKind::String => format!("s{RECEIVER}"),
                JitKind::Dynamic => return None,
                JitKind::Array => format!("{}{}", array_prefix(&receiver)?, RECEIVER),
                JitKind::Dictionary => {
                    format!("{}{}", dictionary_prefix(&receiver)?, RECEIVER)
                }
            };
            let mut continuation_parameters = parameters.clone();
            continuation_parameters.insert(RECEIVER.into(), receiver_token.clone());
            let mut continuation_locals = locals.clone();
            if let Some(source_path) = member_path(&source) {
                for (path, operation) in locals {
                    let Some(suffix) = path.strip_prefix(&format!("{source_path}.")) else {
                        continue;
                    };
                    let Some(operation_suffix) = operation.strip_prefix(receiver.as_slice()) else {
                        continue;
                    };
                    let mut translated = vec![receiver_token.clone()];
                    translated.extend_from_slice(operation_suffix);
                    continuation_locals.insert(format!("{RECEIVER}.{suffix}"), translated);
                }
            }
            let mut continuation = Vec::new();
            encode_expression(
                &ordinary,
                &continuation_parameters,
                &continuation_locals,
                context,
                &mut continuation,
            )?;
            if continuation.first()? != &receiver_token {
                return None;
            }
            continuation.remove(0);
            Some(OptionalChainOperation {
                receiver_presence: None,
                receiver,
                continuation,
            })
        }

        fn encode_fixed_computed_field(
            path: &str,
            key: &Expr,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            match key {
                Expr::Paren(parenthesized) => encode_fixed_computed_field(
                    path,
                    parenthesized.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                ),
                Expr::Lit(Lit::Str(property)) => {
                    let field = format!("{path}.{}", property.value.to_string_lossy());
                    locals
                        .get(&field)
                        .cloned()
                        .or_else(|| parameters.get(&field).map(|value| vec![value.clone()]))
                }
                Expr::Cond(conditional) => {
                    let mut output = Vec::new();
                    encode_condition(
                        conditional.test.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut output,
                    )?;
                    let mut consequent = encode_fixed_computed_field(
                        path,
                        conditional.cons.as_ref(),
                        parameters,
                        locals,
                        context,
                    )?;
                    let mut alternate = encode_fixed_computed_field(
                        path,
                        conditional.alt.as_ref(),
                        parameters,
                        locals,
                        context,
                    )?;
                    normalize_callable_branches([&mut consequent, &mut alternate])?;
                    output.push("if".into());
                    output.extend(consequent);
                    output.push("else".into());
                    output.extend(alternate);
                    output.push("end".into());
                    Some(output)
                }
                _ => None,
            }
        }

        fn encode_runtime_computed_field(
            path: &str,
            key: &Expr,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            fn branches(
                fields: &[(String, Vec<String>)],
                kind: JitKind,
                output: &mut Vec<String>,
            ) -> Option<()> {
                let Some(((name, value), remaining)) = fields.split_first() else {
                    output.push(
                        match kind {
                            JitKind::Number => "absentn",
                            JitKind::Boolean => "absentb",
                            JitKind::String => "absents",
                            JitKind::Array => "absenta",
                            JitKind::Dictionary => "absentd",
                            JitKind::Dynamic => "absentdyn",
                        }
                        .into(),
                    );
                    return Some(());
                };
                output.push("dup".into());
                encode_string(name, output)?;
                output.extend([
                    "strcmp".into(),
                    "c0000000000000000".into(),
                    "==".into(),
                    "if".into(),
                ]);
                output.extend(value.iter().cloned());
                output.push("else".into());
                branches(remaining, kind, output)?;
                output.push("end".into());
                Some(())
            }

            let prefix = format!("{path}.");
            let mut fields = locals
                .iter()
                .filter_map(|(name, value)| {
                    let field = name.strip_prefix(&prefix)?;
                    (!field.contains('.')).then(|| (field.to_owned(), value.clone()))
                })
                .chain(parameters.iter().filter_map(|(name, value)| {
                    let field = name.strip_prefix(&prefix)?;
                    (!field.contains('.')).then(|| (field.to_owned(), vec![value.clone()]))
                }))
                .collect::<Vec<_>>();
            fields.sort_by(|left, right| left.0.cmp(&right.0));
            fields.dedup_by(|left, right| left.0 == right.0);
            let kind = fields
                .iter()
                .map(|(_, value)| jit_expression_kind(value).map(|result| result.0))
                .collect::<Option<Vec<_>>>()?;
            let expected = *kind.first()?;
            if kind.iter().any(|kind| *kind != expected) {
                return None;
            }
            let mut output = Vec::new();
            encode_expression(key, parameters, locals, context, &mut output)?;
            if jit_expression_kind(&output)?.0 != JitKind::String {
                return None;
            }
            branches(&fields, expected, &mut output)?;
            output.push("nip".into());
            Some(output)
        }

        fn encode_fixed_field_assignment(
            path: &str,
            key: &Expr,
            value: &Expr,
            assignment: AssignOp,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            match key {
                Expr::Paren(parenthesized) => encode_fixed_field_assignment(
                    path,
                    parenthesized.expr.as_ref(),
                    value,
                    assignment,
                    parameters,
                    locals,
                    context,
                ),
                Expr::Cond(conditional) => {
                    let mut output = Vec::new();
                    encode_condition(
                        conditional.test.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut output,
                    )?;
                    let mut consequent = encode_fixed_field_assignment(
                        path,
                        conditional.cons.as_ref(),
                        value,
                        assignment,
                        parameters,
                        locals,
                        context,
                    )?;
                    let mut alternate = encode_fixed_field_assignment(
                        path,
                        conditional.alt.as_ref(),
                        value,
                        assignment,
                        parameters,
                        locals,
                        context,
                    )?;
                    normalize_callable_branches([&mut consequent, &mut alternate])?;
                    output.push("if".into());
                    output.extend(consequent);
                    output.push("else".into());
                    output.extend(alternate);
                    output.push("end".into());
                    Some(output)
                }
                Expr::Lit(Lit::Str(property)) => {
                    let field = format!("{path}.{}", property.value.to_string_lossy());
                    let operation = locals
                        .get(&field)
                        .cloned()
                        .or_else(|| parameters.get(&field).map(|value| vec![value.clone()]))?;
                    let (getter, receiver) = operation.split_last()?;
                    if matches!(assignment, AssignOp::AndAssign | AssignOp::OrAssign) {
                        let kind = jit_expression_kind(&operation)?.0;
                        let tagged = getter.starts_with("objopt")
                            || getter.starts_with("objnullable")
                            || getter.starts_with("objnull");
                        let preserve_absence = getter.starts_with("objnull")
                            && !getter.starts_with("objnullable");
                        if !matches!(kind, JitKind::Number | JitKind::Boolean | JitKind::String) {
                            return None;
                        }
                        let mut assigned = encode_fixed_field_assignment(
                            path,
                            key,
                            value,
                            AssignOp::Assign,
                            parameters,
                            locals,
                            context,
                        )?;
                        let mut current = operation;
                        normalize_callable_branches([&mut current, &mut assigned])?;
                        if tagged {
                            let absent = match (kind, preserve_absence) {
                                (JitKind::Number, false) => "absentn",
                                (JitKind::Boolean, false) => "absentb",
                                (JitKind::String, false) => "absents",
                                (JitKind::Number, true) => "keepabsentn",
                                (JitKind::Boolean, true) => "keepabsentb",
                                (JitKind::String, true) => "keepabsents",
                                _ => unreachable!(),
                            };
                            let fallback = assigned.clone();
                            current.push("ifpresent".into());
                            current.push("dup".into());
                            match kind {
                                JitKind::Number => current.push("asbool".into()),
                                JitKind::String => current.push("strbool".into()),
                                JitKind::Boolean => {}
                                _ => unreachable!(),
                            }
                            current.push(
                                if assignment == AssignOp::AndAssign {
                                    "&&"
                                } else {
                                    "||"
                                }
                                .into(),
                            );
                            current.extend(assigned);
                            current.push("end".into());
                            current.push("else".into());
                            if assignment == AssignOp::OrAssign {
                                current.extend(fallback);
                            } else {
                                current.push(absent.into());
                            }
                            current.push("end".into());
                            return Some(current);
                        }
                        current.push("dup".into());
                        match kind {
                            JitKind::Number => current.push("asbool".into()),
                            JitKind::String => current.push("strbool".into()),
                            JitKind::Boolean => {}
                            _ => unreachable!(),
                        }
                        current.push(
                            if assignment == AssignOp::AndAssign {
                                "&&"
                            } else {
                                "||"
                            }
                            .into(),
                        );
                        current.extend(assigned);
                        current.push("end".into());
                        return Some(current);
                    }
                    if assignment == AssignOp::NullishAssign {
                        let tagged = getter.starts_with("objopt")
                            || getter.starts_with("objnullable")
                            || getter.starts_with("objnull");
                        if !tagged {
                            return Some(operation);
                        }
                        let mut assigned = encode_fixed_field_assignment(
                            path,
                            key,
                            value,
                            AssignOp::Assign,
                            parameters,
                            locals,
                            context,
                        )?;
                        let mut present = operation;
                        normalize_callable_branches([&mut present, &mut assigned])?;
                        present.push("ifpresent".into());
                        present.push("else".into());
                        present.extend(assigned);
                        present.push("end".into());
                        return Some(present);
                    }
                    if assignment != AssignOp::Assign {
                        if let Some((semantic, offset)) = getter
                            .strip_prefix("objoptn")
                            .map(|offset| ('o', offset))
                            .or_else(|| {
                                getter
                                    .strip_prefix("objnullablen")
                                    .map(|offset| ('l', offset))
                            })
                            .or_else(|| {
                                getter.strip_prefix("objnulln").map(|offset| ('n', offset))
                            })
                        {
                            let operation = match assignment {
                                AssignOp::AddAssign => 'a',
                                AssignOp::SubAssign => 's',
                                AssignOp::MulAssign => 'm',
                                AssignOp::DivAssign => 'd',
                                AssignOp::ModAssign => 'r',
                                AssignOp::LShiftAssign => 'l',
                                AssignOp::RShiftAssign => 'h',
                                AssignOp::ZeroFillRShiftAssign => 'u',
                                AssignOp::BitOrAssign => 'o',
                                AssignOp::BitXorAssign => 'x',
                                AssignOp::BitAndAssign => 'b',
                                AssignOp::ExpAssign => 'p',
                                AssignOp::Assign
                                | AssignOp::AndAssign
                                | AssignOp::OrAssign
                                | AssignOp::NullishAssign => return None,
                            };
                            let offset = offset.parse::<u16>().ok()?;
                            let mut encoded = Vec::new();
                            encode_expression(
                                value,
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            let mut output = receiver.to_vec();
                            append_number(encoded, &mut output)?;
                            output.push(format!("objca{semantic}{operation}{offset}"));
                            return Some(output);
                        }
                    }
                    let (prefix, expected, offset, present_tag) = if getter
                        .strip_prefix("objn")
                        .is_some_and(|offset| offset.parse::<u16>().is_ok())
                    {
                        (
                            "objsetn",
                            JitKind::Number,
                            getter.strip_prefix("objn")?.parse::<u16>().ok()?,
                            None,
                        )
                    } else if getter
                        .strip_prefix("objb")
                        .is_some_and(|offset| offset.parse::<u16>().is_ok())
                    {
                        (
                            "objsetb",
                            JitKind::Boolean,
                            getter.strip_prefix("objb")?.parse::<u16>().ok()?,
                            None,
                        )
                    } else if getter
                        .strip_prefix("objs")
                        .is_some_and(|offset| offset.parse::<u16>().is_ok())
                    {
                        (
                            "objsets",
                            JitKind::String,
                            getter.strip_prefix("objs")?.parse::<u16>().ok()?,
                            None,
                        )
                    } else if assignment == AssignOp::Assign {
                        let (encoded, present_tag) = getter
                            .strip_prefix("objopt")
                            .map(|offset| (offset, 1u8))
                            .or_else(|| {
                                getter
                                    .strip_prefix("objnullable")
                                    .map(|offset| (offset, 1u8))
                            })
                            .or_else(|| getter.strip_prefix("objnull").map(|offset| (offset, 0u8)))?;
                        let (kind, offset) = encoded.split_at(1);
                        let offset = offset.parse::<u16>().ok()?;
                        let (prefix, expected, payload_offset) = match kind {
                            "n" => ("objsetn", JitKind::Number, offset.checked_add(8)?),
                            "b" => ("objsetb", JitKind::Boolean, offset.checked_add(1)?),
                            "s" => ("objsets", JitKind::String, offset.checked_add(8)?),
                            _ => return None,
                        };
                        (prefix, expected, payload_offset, Some((offset, present_tag)))
                    } else {
                        return None;
                    };
                    let mut encoded = Vec::new();
                    encode_expression(value, parameters, locals, context, &mut encoded)?;
                    let mut output = receiver.to_vec();
                    if assignment == AssignOp::Assign {
                        if !jit_kind_compatible(jit_expression_kind(&encoded)?.0, expected) {
                            return None;
                        }
                        output.extend(encoded);
                    } else if assignment == AssignOp::AddAssign && expected == JitKind::String {
                        output.push("dup".into());
                        output.push(getter.clone());
                        append_string(encoded, &mut output)?;
                        output.push("concat".into());
                    } else {
                        if expected != JitKind::Number
                            || jit_expression_kind(&encoded)?.0 == JitKind::Array
                        {
                            return None;
                        }
                        output.push("dup".into());
                        output.push(getter.clone());
                        append_number(encoded, &mut output)?;
                        output.push(
                            match assignment {
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
                                AssignOp::Assign
                                | AssignOp::AndAssign
                                | AssignOp::OrAssign
                                | AssignOp::NullishAssign => return None,
                            }
                            .into(),
                        );
                    }
                    output.extend([
                        "dup2".into(),
                        format!("{prefix}{offset}"),
                    ]);
                    if let Some((tag_offset, tag)) = present_tag {
                        output.push(format!("c{:016x}", f64::from(tag).to_bits()));
                        output.push(format!("objsetb{tag_offset}"));
                    }
                    output.extend(["drop".into(), "nip".into()]);
                    Some(output)
                }
                _ => None,
            }
        }

        fn encode_fixed_field_update(
            path: &str,
            key: &Expr,
            update: UpdateOp,
            prefix: bool,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            match key {
                Expr::Paren(parenthesized) => encode_fixed_field_update(
                    path,
                    parenthesized.expr.as_ref(),
                    update,
                    prefix,
                    parameters,
                    locals,
                    context,
                ),
                Expr::Cond(conditional) => {
                    let mut output = Vec::new();
                    encode_condition(
                        conditional.test.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut output,
                    )?;
                    let mut consequent = encode_fixed_field_update(
                        path,
                        conditional.cons.as_ref(),
                        update,
                        prefix,
                        parameters,
                        locals,
                        context,
                    )?;
                    let mut alternate = encode_fixed_field_update(
                        path,
                        conditional.alt.as_ref(),
                        update,
                        prefix,
                        parameters,
                        locals,
                        context,
                    )?;
                    normalize_callable_branches([&mut consequent, &mut alternate])?;
                    output.push("if".into());
                    output.extend(consequent);
                    output.push("else".into());
                    output.extend(alternate);
                    output.push("end".into());
                    Some(output)
                }
                Expr::Lit(Lit::Str(property)) => {
                    let field = format!("{path}.{}", property.value.to_string_lossy());
                    let operation = locals
                        .get(&field)
                        .cloned()
                        .or_else(|| parameters.get(&field).map(|value| vec![value.clone()]))?;
                    let (getter, receiver) = operation.split_last()?;
                    if let Some((semantic, offset)) = getter
                        .strip_prefix("objoptn")
                        .map(|offset| ('o', offset))
                        .or_else(|| {
                            getter
                                .strip_prefix("objnullablen")
                                .map(|offset| ('l', offset))
                        })
                        .or_else(|| getter.strip_prefix("objnulln").map(|offset| ('n', offset)))
                    {
                        let offset = offset.parse::<u16>().ok()?;
                        let operation = match update {
                            UpdateOp::PlusPlus => 'i',
                            UpdateOp::MinusMinus => 'd',
                        };
                        let result = if prefix { 'p' } else { 'o' };
                        let mut output = receiver.to_vec();
                        output.push(format!("objup{semantic}{operation}{result}{offset}"));
                        return Some(output);
                    }
                    let offset = getter.strip_prefix("objn")?.parse::<u16>().ok()?;
                    let mut output = receiver.to_vec();
                    output.extend(["dup".into(), getter.clone()]);
                    if !prefix {
                        output.push("dup2".into());
                    }
                    output.push(format!("c{:016x}", 1.0f64.to_bits()));
                    output.push(
                        match update {
                            UpdateOp::PlusPlus => "+",
                            UpdateOp::MinusMinus => "-",
                        }
                        .into(),
                    );
                    if prefix {
                        output.push("dup2".into());
                    }
                    output.extend([
                        format!("objsetn{offset}"),
                        "drop".into(),
                        "nip".into(),
                    ]);
                    Some(output)
                }
                _ => None,
            }
        }

        match expression {
            Expr::Ident(identifier) if locals.contains_key(identifier.sym.as_ref()) => {
                output.extend(locals.get(identifier.sym.as_ref())?.iter().cloned());
            }
            Expr::Ident(identifier) if parameters.contains_key(identifier.sym.as_ref()) => {
                let value = parameters.get(identifier.sym.as_ref())?;
                if value.starts_with("optional:") {
                    return None;
                }
                output.push(value.clone());
            }
            Expr::Ident(_) => output.push(format!(
                "c{:016x}",
                numeric_constant(expression, parameters, locals, context.helpers)?.to_bits()
            )),
            Expr::Lit(Lit::Num(number)) => {
                output.push(format!("c{:016x}", number.value.to_bits()));
            }
            Expr::Lit(Lit::Bool(boolean)) => {
                output.push(format!(
                    "c{:016x}",
                    f64::from(u8::from(boolean.value)).to_bits()
                ));
            }
            Expr::Lit(Lit::Str(string)) => {
                let string = string.value.to_string_lossy();
                encode_string(&string, output)?;
            }
            Expr::Tpl(template) if template.quasis.len() == template.exprs.len() + 1 => {
                let mut emitted = false;
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let value = quasi
                        .cooked
                        .as_ref()
                        .map(|value| value.to_string_lossy())
                        .unwrap_or_else(|| quasi.raw.to_string().into());
                    if !value.is_empty() {
                        encode_string(&value, output)?;
                        if emitted {
                            output.push("concat".into());
                        }
                        emitted = true;
                    }
                    if let Some(expression) = template.exprs.get(index) {
                        let mut encoded = Vec::new();
                        encode_expression(expression, parameters, locals, context, &mut encoded)?;
                        append_string(encoded, output)?;
                        if emitted {
                            output.push("concat".into());
                        }
                        emitted = true;
                    }
                }
                if !emitted {
                    output.push("t".into());
                }
            }
            Expr::Array(array) => {
                output.push("arrayempty".into());
                let mut prefix = None;
                for element in &array.elems {
                    let element = element.as_ref()?;
                    append_array_element(
                        element,
                        parameters,
                        locals,
                        context,
                        &mut prefix,
                        output,
                    )?;
                }
            }
            Expr::Object(object) => {
                let mut entries = Vec::new();
                let mut kind = None;
                for property in &object.props {
                    let (key, value) = object_property(property)?;
                    let mut encoded = Vec::new();
                    match value {
                        ObjectReturnValue::Expression(expression) => encode_expression(
                            expression,
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?,
                        ObjectReturnValue::Shorthand(identifier) => encode_expression(
                            &Expr::Ident(identifier.clone()),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?,
                    }
                    let value_kind = jit_expression_kind(&encoded)?.0;
                    if matches!(value_kind, JitKind::Array | JitKind::Dictionary)
                        || kind.replace(value_kind).is_some_and(|kind| kind != value_kind)
                    {
                        return None;
                    }
                    entries.push((key, encoded));
                }
                let prefix = match kind? {
                    JitKind::Number => "dn",
                    JitKind::Boolean => "db",
                    JitKind::String => "ds",
                    JitKind::Dynamic | JitKind::Array | JitKind::Dictionary => return None,
                };
                output.push(format!("{prefix}empty"));
                for (key, value) in entries {
                    let mut encoded_key = Vec::new();
                    encode_string(&key, &mut encoded_key)?;
                    let encoded_key = encoded_key.pop()?.strip_prefix('t')?.to_owned();
                    output.extend(value);
                    output.push(format!("{prefix}put{encoded_key}"));
                }
            }
            Expr::Member(member) => {
                let common_array_receiver = matches!(&member.prop, MemberProp::Computed(_))
                    && matches!(member.obj.as_ref(), Expr::Ident(identifier)
                        if locals.get(identifier.sym.as_ref()).is_some_and(|value| {
                            matches!(value.last().map(String::as_str), Some("untagarrayn" | "untagarrayb" | "untagarrays"))
                        }));
                let mut object_field = (!common_array_receiver)
                    .then(|| member_path(expression))
                    .flatten()
                    .and_then(|path| {
                        locals
                            .get(&path)
                            .cloned()
                            .or_else(|| parameters.get(&path).map(|field| vec![field.clone()]))
                    });
                if object_field.is_none() {
                    if let MemberProp::Computed(computed) = &member.prop {
                        if let Some(path) = member_path(member.obj.as_ref()) {
                            object_field = encode_fixed_computed_field(
                                &path,
                                computed.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                            )
                            .or_else(|| {
                                encode_runtime_computed_field(
                                    &path,
                                    computed.expr.as_ref(),
                                    parameters,
                                    locals,
                                    context,
                                )
                            });
                        }
                    }
                }
                if matches!(
                    (member.obj.as_ref(), &member.prop),
                    (Expr::Ident(object), MemberProp::Ident(property))
                        if object.sym == "process"
                            && matches!(property.sym.as_ref(), "pid" | "ppid")
                            && !parameters.contains_key("process")
                            && !locals.contains_key("process")
                ) {
                    let MemberProp::Ident(property) = &member.prop else {
                        unreachable!();
                    };
                    output.push(format!("process{}", property.sym));
                } else if let Some(field) = object_field {
                    output.extend(field);
                } else if let MemberProp::Computed(computed) = &member.prop {
                    let mut receiver = Vec::new();
                    encode_expression(
                        member.obj.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut receiver,
                    )?;
                    match jit_expression_kind(&receiver)?.0 {
                        JitKind::Array => {
                            let prefix = array_prefix(&receiver)?;
                            output.extend(receiver);
                            encode_number(
                                computed.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                            output.push(format!("{prefix}get"));
                        }
                        JitKind::Dictionary => {
                            let prefix = dictionary_prefix(&receiver)?;
                            let mut key = Vec::new();
                            encode_expression(
                                computed.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut key,
                            )?;
                            if jit_expression_kind(&key)?.0 != JitKind::String {
                                return None;
                            }
                            output.extend(receiver);
                            output.extend(key);
                            output.push(format!("{prefix}get"));
                        }
                        _ => return None,
                    }
                } else if let MemberProp::Ident(property) = &member.prop {
                    let mut receiver = Vec::new();
                    let encoded_receiver = encode_expression(
                        member.obj.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut receiver,
                    )
                    .is_some();
                    if receiver.len() == 1 && jit_dynamic_array_argument(&receiver[0]) {
                        receiver.push("untagarray".into());
                    }
                    if encoded_receiver
                        && jit_expression_kind(&receiver)?.0 == JitKind::Dictionary
                    {
                        let prefix = dictionary_prefix(&receiver)?;
                        output.extend(receiver);
                        encode_string(property.sym.as_ref(), output)?;
                        output.push(format!("{prefix}get"));
                    } else if property.sym == "length" {
                        let operation = match jit_expression_kind(&receiver)?.0 {
                            JitKind::String => "strlen",
                            JitKind::Array => "arraylen",
                            JitKind::Dynamic
                                if receiver.len() == 1
                                    && jit_dynamic_array_argument(&receiver[0]) =>
                            {
                                "dynarraylen"
                            }
                            _ => return None,
                        };
                        output.extend(receiver);
                        output.push(operation.into());
                    } else {
                        output.push(format!(
                            "c{:016x}",
                            numeric_constant(expression, parameters, locals, context.helpers)?
                                .to_bits()
                        ));
                    }
                } else {
                    output.push(format!(
                        "c{:016x}",
                        numeric_constant(expression, parameters, locals, context.helpers)?.to_bits()
                    ));
                }
            }
            Expr::Assign(assignment) => {
                let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left
                else {
                    return None;
                };
                let key = match &target.prop {
                    MemberProp::Ident(property) => {
                        Expr::Lit(Lit::Str(property.sym.to_string().into()))
                    }
                    MemberProp::Computed(property) => property.expr.as_ref().clone(),
                    MemberProp::PrivateName(_) => return None,
                };
                if let Some(path) = member_path(target.obj.as_ref()) {
                    if let Some(encoded) = encode_fixed_field_assignment(
                        &path,
                        &key,
                        assignment.right.as_ref(),
                        assignment.op,
                        parameters,
                        locals,
                        context,
                    ) {
                        output.extend(encoded);
                        return (output.len() <= 256).then_some(());
                    }
                }
                let mut receiver = Vec::new();
                encode_expression(
                    target.obj.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut receiver,
                )?;
                let dynamic_array = receiver.last().is_some_and(|token| token == "untagarray");
                if dynamic_array {
                    receiver.pop();
                }
                let receiver_kind = if dynamic_array {
                    JitKind::Dynamic
                } else {
                    jit_expression_kind(&receiver)?.0
                };
                let prefix = match receiver_kind {
                    JitKind::Array => array_prefix(&receiver)?,
                    JitKind::Dictionary => dictionary_prefix(&receiver)?,
                    JitKind::Dynamic if dynamic_array => "dynamic",
                    _ => return None,
                };
                let expected = match prefix.as_bytes().get(1) {
                    Some(b'n') => JitKind::Number,
                    Some(b's') => JitKind::String,
                    Some(b'b') => JitKind::Boolean,
                    _ if dynamic_array => JitKind::Dynamic,
                    _ => return None,
                };
                let local_set = if assignment.op == AssignOp::Assign {
                    match target.obj.as_ref() {
                        Expr::Ident(identifier) => locals
                            .get(identifier.sym.as_ref())
                            .and_then(|tokens| tokens.first())
                            .and_then(|local| {
                                local
                                    .starts_with(&format!("{prefix}l"))
                                    .then(|| loop_local_index(local))
                                    .flatten()
                            })
                            .map(|index| format!("{prefix}lset{index}")),
                        _ => None,
                    }
                } else {
                    None
                };
                if local_set.is_none() {
                    output.extend(receiver);
                }
                match (&target.prop, receiver_kind) {
                    (MemberProp::Computed(index), JitKind::Array | JitKind::Dynamic) => {
                        encode_number(index.expr.as_ref(), parameters, locals, context, output)?;
                    }
                    (MemberProp::Computed(key), JitKind::Dictionary) => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            key.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        if jit_expression_kind(&encoded)?.0 != JitKind::String {
                            return None;
                        }
                        output.extend(encoded);
                    }
                    (MemberProp::Ident(key), JitKind::Dictionary) => {
                        encode_string(key.sym.as_ref(), output)?;
                    }
                    _ => return None,
                }
                if assignment.op != AssignOp::Assign {
                    output.push("dup2".into());
                    output.push(format!("{prefix}get"));
                }
                let mut value = Vec::new();
                encode_expression(assignment.right.as_ref(), parameters, locals, context, &mut value)?;
                if assignment.op == AssignOp::Assign {
                    if jit_expression_kind(&value)?.0 != expected {
                        return None;
                    }
                    output.extend(value);
                } else if assignment.op == AssignOp::AddAssign && expected == JitKind::String {
                    append_string(value, output)?;
                    output.push("concat".into());
                } else {
                    if expected != JitKind::Number || jit_expression_kind(&value)?.0 == JitKind::Array {
                        return None;
                    }
                    append_number(value, output)?;
                    if assignment.op != AssignOp::Assign {
                        output.push(
                            match assignment.op {
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
                }
                output.push(if dynamic_array {
                    "dynarrayset".into()
                } else {
                    local_set.unwrap_or_else(|| format!("{prefix}set"))
                });
            }
            Expr::Update(update) => {
                let Expr::Member(target) = update.arg.as_ref() else {
                    return None;
                };
                let key = match &target.prop {
                    MemberProp::Ident(property) => {
                        Expr::Lit(Lit::Str(property.sym.to_string().into()))
                    }
                    MemberProp::Computed(property) => property.expr.as_ref().clone(),
                    MemberProp::PrivateName(_) => return None,
                };
                if let Some(path) = member_path(target.obj.as_ref()) {
                    if let Some(encoded) = encode_fixed_field_update(
                        &path,
                        &key,
                        update.op,
                        update.prefix,
                        parameters,
                        locals,
                        context,
                    ) {
                        output.extend(encoded);
                        return (output.len() <= 256).then_some(());
                    }
                }
                let mut receiver = Vec::new();
                encode_expression(
                    target.obj.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut receiver,
                )?;
                let receiver_kind = jit_expression_kind(&receiver)?.0;
                let prefix = match receiver_kind {
                    JitKind::Array if array_prefix(&receiver)? == "rn" => "rn",
                    JitKind::Dictionary if dictionary_prefix(&receiver)? == "dn" => "dn",
                    _ => return None,
                };
                output.extend(receiver);
                match (&target.prop, receiver_kind) {
                    (MemberProp::Computed(index), JitKind::Array) => {
                        encode_number(index.expr.as_ref(), parameters, locals, context, output)?;
                    }
                    (MemberProp::Computed(key), JitKind::Dictionary) => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            key.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        if jit_expression_kind(&encoded)?.0 != JitKind::String {
                            return None;
                        }
                        output.extend(encoded);
                    }
                    (MemberProp::Ident(key), JitKind::Dictionary) => {
                        encode_string(key.sym.as_ref(), output)?;
                    }
                    _ => return None,
                }
                if prefix != "rn" && prefix != "dn" {
                    return None;
                }
                output.push("dup2".into());
                output.push(format!("{prefix}get"));
                if !update.prefix {
                    output.push("dup".into());
                }
                output.push(format!("c{:016x}", 1.0f64.to_bits()));
                output.push(
                    match update.op {
                        UpdateOp::PlusPlus => "+",
                        UpdateOp::MinusMinus => "-",
                    }
                    .into(),
                );
                output.push(
                    if update.prefix {
                        format!("{prefix}set")
                    } else {
                        format!("{prefix}postset")
                    },
                );
            }
            Expr::Unary(unary) if unary.op == UnaryOp::Delete => {
                let Expr::Member(target) = unary.arg.as_ref() else {
                    return None;
                };
                let mut receiver = Vec::new();
                encode_expression(
                    target.obj.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut receiver,
                )?;
                if jit_expression_kind(&receiver)?.0 != JitKind::Dictionary {
                    return None;
                }
                output.extend(receiver);
                match &target.prop {
                    MemberProp::Computed(key) => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            key.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        if jit_expression_kind(&encoded)?.0 != JitKind::String {
                            return None;
                        }
                        output.extend(encoded);
                    }
                    MemberProp::Ident(key) => encode_string(key.sym.as_ref(), output)?,
                    _ => return None,
                }
                output.push("ddelete".into());
            }
            Expr::Unary(unary)
                if matches!(
                    unary.op,
                    UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Tilde | UnaryOp::Bang
                ) =>
            {
                if unary.op == UnaryOp::Bang {
                    let mut encoded = Vec::new();
                    encode_expression(unary.arg.as_ref(), parameters, locals, context, &mut encoded)?;
                    append_boolean(encoded, output)?;
                } else {
                    encode_number(unary.arg.as_ref(), parameters, locals, context, output)?;
                }
                match unary.op {
                    UnaryOp::Minus => output.push("neg".into()),
                    UnaryOp::Tilde => output.push("bnot".into()),
                    UnaryOp::Bang => output.push("boolnot".into()),
                    _ => {}
                }
            }
            Expr::Unary(unary) if unary.op == UnaryOp::TypeOf => {
                let mut encoded = Vec::new();
                encode_expression(unary.arg.as_ref(), parameters, locals, context, &mut encoded)?;
                let operation = match jit_expression_kind(&encoded)?.0 {
                    JitKind::Number => "typeofnumber",
                    JitKind::Boolean => "typeofboolean",
                    JitKind::String => "typeofstring",
                    JitKind::Dynamic => "typeofdynamic",
                    JitKind::Array | JitKind::Dictionary => "typeofobject",
                };
                output.extend(encoded);
                output.push(operation.into());
            }
            Expr::Paren(parenthesized) => {
                encode_expression(parenthesized.expr.as_ref(), parameters, locals, context, output)?;
            }
            Expr::Bin(binary) if binary.op == BinaryOp::Add => {
                let mut left = Vec::new();
                let mut right = Vec::new();
                encode_expression(binary.left.as_ref(), parameters, locals, context, &mut left)?;
                encode_expression(binary.right.as_ref(), parameters, locals, context, &mut right)?;
                append_add(left, right, output)?;
            }
            Expr::Bin(binary)
                if matches!(
                    binary.op,
                    BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                        | BinaryOp::LShift
                        | BinaryOp::RShift
                        | BinaryOp::ZeroFillRShift
                        | BinaryOp::Exp
                ) =>
            {
                for operand in [&binary.left, &binary.right] {
                    encode_number(operand.as_ref(), parameters, locals, context, output)?;
                }
                output.push(
                    match binary.op {
                        BinaryOp::Sub => "-",
                        BinaryOp::Mul => "*",
                        BinaryOp::Div => "/",
                        BinaryOp::Mod => "%",
                        BinaryOp::BitAnd => "band",
                        BinaryOp::BitOr => "bor",
                        BinaryOp::BitXor => "bxor",
                        BinaryOp::LShift => "shl",
                        BinaryOp::RShift => "shr",
                        BinaryOp::ZeroFillRShift => "ushr",
                        BinaryOp::Exp => "pow",
                        _ => unreachable!(),
                    }
                    .into(),
                );
            }
            Expr::Bin(binary)
                if matches!(binary.op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) =>
            {
                let mut left = Vec::new();
                let mut right = Vec::new();
                encode_expression(binary.left.as_ref(), parameters, locals, context, &mut left)?;
                encode_expression(binary.right.as_ref(), parameters, locals, context, &mut right)?;
                let left_kind = jit_expression_kind(&left)?.0;
                let right_kind = jit_expression_kind(&right)?.0;
                let kind = if left_kind == right_kind {
                    left_kind
                } else if left_kind == JitKind::Dynamic
                    && matches!(right_kind, JitKind::Number | JitKind::Boolean | JitKind::String)
                {
                    let mut tagged = Vec::new();
                    append_dynamic(right, &mut tagged)?;
                    right = tagged;
                    JitKind::Dynamic
                } else if right_kind == JitKind::Dynamic
                    && matches!(left_kind, JitKind::Number | JitKind::Boolean | JitKind::String)
                {
                    let mut tagged = Vec::new();
                    append_dynamic(left, &mut tagged)?;
                    left = tagged;
                    JitKind::Dynamic
                } else {
                    return None;
                };
                if matches!(kind, JitKind::Array | JitKind::Dictionary) {
                    // Arrays and dictionaries are unconditionally truthy in JavaScript
                    // (even when empty), so the runtime value never needs checking:
                    // `a || b` always yields `a` and `a && b` always yields `b`. Both
                    // sides were already validated as encodable above; only the
                    // statically-selected side's tokens need to survive into the
                    // output, and the other side's evaluation (side effects
                    // included) is correctly never observed at runtime either way.
                    output.extend(if binary.op == BinaryOp::LogicalAnd {
                        right
                    } else {
                        left
                    });
                    return Some(());
                }
                output.extend(left);
                output.push("dup".into());
                match kind {
                    JitKind::Number => output.push("asbool".into()),
                    JitKind::String => output.push("strbool".into()),
                    JitKind::Boolean => {}
                    JitKind::Dynamic => output.push("dynbool".into()),
                    JitKind::Array | JitKind::Dictionary => unreachable!(),
                }
                output.push(if binary.op == BinaryOp::LogicalAnd {
                    "&&"
                } else {
                    "||"
                }
                .into());
                output.extend(right);
                output.push("end".into());
            }
            Expr::Bin(binary) if binary.op == BinaryOp::NullishCoalescing => {
                let mut operands = Vec::new();
                flatten_nullish(expression, &mut operands);
                let mut selected = Vec::new();
                encode_expression(
                    operands.pop()?,
                    parameters,
                    locals,
                    context,
                    &mut selected,
                )?;
                while let Some(operand) = operands.pop() {
                    let fallback = selected;
                    if let Some((presence, value)) = optional_tokens(operand, parameters) {
                        let mut present = vec![value.into()];
                        let mut fallback = fallback;
                        normalize_callable_branches([&mut present, &mut fallback])?;
                        selected = presence;
                        selected.extend(["asbool".into(), "if".into()]);
                        selected.extend(present);
                        selected.push("else".into());
                        selected.extend(fallback);
                        selected.push("end".into());
                    } else if let Expr::OptChain(chain) = operand {
                        let optional =
                            optional_chain_operation(chain, parameters, locals, context)?;
                        if let Some(presence) = optional.receiver_presence {
                            selected = presence;
                            selected.extend(["asbool".into(), "if".into()]);
                            selected.extend(optional.receiver.clone());
                            selected.extend(optional.continuation);
                            if jit_operation_may_be_absent(&optional.receiver) {
                                selected.push("ifpresent".into());
                                selected.push("else".into());
                                selected.extend(fallback.clone());
                                selected.push("end".into());
                            }
                            selected.push("else".into());
                            selected.extend(fallback);
                            selected.push("end".into());
                        } else {
                            selected = optional.receiver;
                            selected.push("ifpresent".into());
                            selected.extend(optional.continuation.clone());
                            if jit_operation_may_be_absent(&optional.continuation) {
                                selected.push("ifpresent".into());
                                selected.push("else".into());
                                selected.extend(fallback.clone());
                                selected.push("end".into());
                            }
                            selected.push("else".into());
                            selected.extend(fallback);
                            selected.push("end".into());
                        }
                    } else {
                        let mut operation = Vec::new();
                        encode_expression(
                            operand,
                            parameters,
                            locals,
                            context,
                            &mut operation,
                        )?;
                        if jit_operation_may_be_absent(&operation) {
                            selected = operation;
                            selected.push("ifpresent".into());
                            selected.push("else".into());
                            selected.extend(fallback);
                            selected.push("end".into());
                        } else {
                            selected = operation;
                        }
                    }
                }
                output.extend(selected);
            }
            Expr::OptChain(chain) => {
                let optional = optional_chain_operation(chain, parameters, locals, context)?;
                let mut operation = optional.receiver.clone();
                operation.extend(optional.continuation.clone());
                let kind = jit_expression_kind(&operation)?.0;
                if let Some(presence) = optional.receiver_presence {
                    output.extend(presence);
                    output.push("asbool".into());
                    output.push("if".into());
                    output.extend(operation);
                } else {
                    output.extend(optional.receiver);
                    output.push("ifpresent".into());
                    output.extend(optional.continuation);
                }
                output.push("else".into());
                output.push(
                    match kind {
                        JitKind::Number => "absentn",
                        JitKind::Boolean => "absentb",
                        JitKind::String => "absents",
                        JitKind::Dynamic => "absentdyn",
                        JitKind::Array => "absenta",
                        JitKind::Dictionary => "absentd",
                    }
                    .into(),
                );
                output.push("end".into());
            }
            Expr::Bin(binary) if binary.op == BinaryOp::In => {
                let mut key = Vec::new();
                encode_expression(
                    binary.left.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut key,
                )?;
                append_string(key, output)?;
                let mut object = Vec::new();
                encode_expression(
                    binary.right.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut object,
                )?;
                if jit_expression_kind(&object)?.0 != JitKind::Dictionary {
                    return None;
                }
                output.extend(object);
                output.push("din".into());
            }
            Expr::Bin(binary)
                if matches!(
                    binary.op,
                    BinaryOp::Lt
                        | BinaryOp::LtEq
                        | BinaryOp::Gt
                        | BinaryOp::GtEq
                        | BinaryOp::EqEq
                        | BinaryOp::EqEqEq
                        | BinaryOp::NotEq
                        | BinaryOp::NotEqEq
                ) =>
            {
                encode_condition(expression, parameters, locals, context, output)?;
            }
            Expr::Call(call) if number_format_method(call, parameters, locals).is_some() => {
                let (operation, receiver) = number_format_method(call, parameters, locals)?;
                let mut encoded = Vec::new();
                encode_expression(receiver, parameters, locals, context, &mut encoded)?;
                let receiver_kind = jit_expression_kind(&encoded)?.0;
                if operation == "toradix" && call.args.is_empty() {
                    match receiver_kind {
                        JitKind::Boolean => {
                            output.extend(encoded);
                            output.push("boolstr".into());
                        }
                        JitKind::String => output.extend(encoded),
                        JitKind::Number => append_string(encoded, output)?,
                        JitKind::Dynamic => append_string(encoded, output)?,
                        JitKind::Array | JitKind::Dictionary => return None,
                    }
                    return Some(());
                }
                if receiver_kind != JitKind::Number {
                    return None;
                }
                match call.args.as_slice() {
                    [] if operation == "tofixed" => {
                        output.extend(encoded);
                        output.push(format!("c{:016x}", 0.0f64.to_bits()));
                        output.push(operation.into());
                    }
                    [] if operation == "toexponential" => {
                        output.extend(encoded);
                        output.push("toexponential0".into());
                    }
                    [] => append_string(encoded, output)?,
                    [argument] => {
                        output.extend(encoded);
                        encode_number(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                        output.push(operation.into());
                    }
                    _ => return None,
                }
            }
            Expr::Call(call)
                if object_assign_call(call, parameters, locals, context.helpers).is_some() =>
            {
                let arguments = object_assign_call(call, parameters, locals, context.helpers)?;
                fn is_empty_object_literal(expression: &Expr) -> bool {
                    match expression {
                        Expr::Paren(parenthesized) => {
                            is_empty_object_literal(parenthesized.expr.as_ref())
                        }
                        Expr::Object(object) => object.props.is_empty(),
                        _ => false,
                    }
                }
                // A bare `{}` argument has no properties to infer a dictionary value
                // type from on its own, so its kind is resolved from whichever
                // sibling argument (target or source) does carry one instead; an
                // empty literal then becomes a same-prefix empty dictionary rather
                // than going through the normal (kind-less) object-literal encoding.
                let prefix = arguments
                    .iter()
                    .filter(|argument| !is_empty_object_literal(argument.expr.as_ref()))
                    .find_map(|argument| {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        (jit_expression_kind(&encoded)?.0 == JitKind::Dictionary)
                            .then_some(())
                            .and_then(|()| dictionary_prefix(&encoded))
                    })?;
                for (index, argument) in arguments.iter().enumerate() {
                    let mut encoded = Vec::new();
                    if is_empty_object_literal(argument.expr.as_ref()) {
                        encoded.push(format!("{prefix}empty"));
                    } else {
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        if jit_expression_kind(&encoded)?.0 != JitKind::Dictionary
                            || dictionary_prefix(&encoded)? != prefix
                        {
                            return None;
                        }
                    }
                    output.extend(encoded);
                    if index > 0 {
                        output.push("dassign".into());
                    }
                }
            }
            Expr::Call(call)
                if object_from_entries_call(call, parameters, locals, context.helpers).is_some() =>
            {
                let entries = object_from_entries_call(call, parameters, locals, context.helpers)?;
                // `Object.fromEntries` most commonly round-trips `Object.entries(dict)`
                // (handled below via `entry_prefix`), but a literal array of
                // statically-keyed pairs can be built directly into a dictionary
                // literal instead - the general array encoder can't represent an
                // array of tuples at all, so this bypasses it entirely rather than
                // trying to make tuple-shaped array elements a general capability.
                fn literal_entries_dictionary(
                    entries: &Expr,
                    parameters: &std::collections::HashMap<String, String>,
                    locals: &std::collections::HashMap<String, Vec<String>>,
                    context: &mut InlineContext<'_>,
                ) -> Option<Vec<String>> {
                    let entries = match entries {
                        Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
                        entries => entries,
                    };
                    let Expr::Array(entries) = entries else {
                        return None;
                    };
                    let mut kind = None;
                    let mut pairs = Vec::new();
                    for element in &entries.elems {
                        let element = element.as_ref()?;
                        if element.spread.is_some() {
                            return None;
                        }
                        let Expr::Array(pair) = element.expr.as_ref() else {
                            return None;
                        };
                        let [Some(key), Some(value)] = pair.elems.as_slice() else {
                            return None;
                        };
                        if key.spread.is_some() || value.spread.is_some() {
                            return None;
                        }
                        let Expr::Lit(Lit::Str(key)) = key.expr.as_ref() else {
                            return None;
                        };
                        let mut encoded_value = Vec::new();
                        encode_expression(
                            value.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded_value,
                        )?;
                        let value_kind = jit_expression_kind(&encoded_value)?.0;
                        if matches!(
                            value_kind,
                            JitKind::Array | JitKind::Dictionary | JitKind::Dynamic
                        ) || kind.replace(value_kind).is_some_and(|kind| kind != value_kind)
                        {
                            return None;
                        }
                        pairs.push((key.value.to_string_lossy().into_owned(), encoded_value));
                    }
                    let prefix = match kind? {
                        JitKind::Number => "dn",
                        JitKind::Boolean => "db",
                        JitKind::String => "ds",
                        JitKind::Dynamic | JitKind::Array | JitKind::Dictionary => {
                            unreachable!()
                        }
                    };
                    let mut output = vec![format!("{prefix}empty")];
                    for (key, value) in pairs {
                        let mut encoded_key = Vec::new();
                        encode_string(&key, &mut encoded_key)?;
                        let encoded_key = encoded_key.pop()?.strip_prefix('t')?.to_owned();
                        output.extend(value);
                        output.push(format!("{prefix}put{encoded_key}"));
                    }
                    Some(output)
                }
                if let Some(literal) =
                    literal_entries_dictionary(entries, parameters, locals, context)
                {
                    output.extend(literal);
                } else {
                    let mut encoded = Vec::new();
                    encode_expression(entries, parameters, locals, context, &mut encoded)?;
                    if jit_expression_kind(&encoded)?.0 != JitKind::Array {
                        return None;
                    }
                    let operation = match entry_prefix(&encoded)? {
                        "dn" => "dnfromentries",
                        "db" => "dbfromentries",
                        "ds" => "dsfromentries",
                        _ => unreachable!(),
                    };
                    output.extend(encoded);
                    output.push(operation.into());
                }
            }
            Expr::Call(call)
                if object_dictionary_call(call, parameters, locals, context.helpers).is_some() =>
            {
                let (operation, object, key) =
                    object_dictionary_call(call, parameters, locals, context.helpers)?;
                let mut encoded = Vec::new();
                encode_expression(object, parameters, locals, context, &mut encoded)?;
                if jit_expression_kind(&encoded)?.0 != JitKind::Dictionary {
                    return None;
                }
                let operation = if matches!(operation, "dvalues" | "dentries") {
                    let entries = operation == "dentries";
                    match dictionary_prefix(&encoded)? {
                        "dn" if !entries => "dnvalues",
                        "db" if !entries => "dbvalues",
                        "ds" if !entries => "dsvalues",
                        "dn" => "dnentries",
                        "db" => "dbentries",
                        "ds" => "dsentries",
                        _ => unreachable!(),
                    }
                } else {
                    operation
                };
                output.extend(encoded);
                if let Some(key) = key {
                    let mut encoded = Vec::new();
                    encode_expression(key, parameters, locals, context, &mut encoded)?;
                    if jit_expression_kind(&encoded)?.0 != JitKind::String {
                        return None;
                    }
                    output.extend(encoded);
                }
                output.push(operation.into());
            }
            Expr::Call(call)
                if object_same_value(call, parameters, locals, context.helpers).is_some() =>
            {
                let (left, right) =
                    object_same_value(call, parameters, locals, context.helpers)?;
                let mut encoded_left = Vec::new();
                let mut encoded_right = Vec::new();
                encode_expression(left, parameters, locals, context, &mut encoded_left)?;
                encode_expression(right, parameters, locals, context, &mut encoded_right)?;
                let left_kind = jit_expression_kind(&encoded_left)?.0;
                let right_kind = jit_expression_kind(&encoded_right)?.0;
                output.extend(encoded_left);
                output.extend(encoded_right);
                output.push(
                    if left_kind != right_kind {
                        "strictfalse"
                    } else {
                        match left_kind {
                            JitKind::Number => "numsame",
                            JitKind::Boolean => "==",
                            JitKind::String => "strsame",
                            JitKind::Dynamic => return None,
                            JitKind::Array | JitKind::Dictionary => "refsame",
                        }
                    }
                    .into(),
                );
            }
            Expr::Call(call)
                if array_constructor(call, parameters, locals, context.helpers).is_some() =>
            {
                match array_constructor(call, parameters, locals, context.helpers)? {
                    "of" => {
                        output.push("arrayempty".into());
                        let mut prefix = None;
                        for argument in &call.args {
                            append_array_element(
                                argument,
                                parameters,
                                locals,
                                context,
                                &mut prefix,
                                output,
                            )?;
                        }
                    }
                    "from" => {
                        let [argument] = call.args.as_slice() else {
                            return None;
                        };
                        if argument.spread.is_some() {
                            return None;
                        }
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        match jit_expression_kind(&encoded)?.0 {
                            JitKind::Array => {
                                output.extend(encoded);
                                output.push(format!("c{:016x}", 0.0f64.to_bits()));
                                output.push(format!("c{:016x}", f64::INFINITY.to_bits()));
                                output.push("arrayslice".into());
                            }
                            JitKind::String => {
                                output.extend(encoded);
                                output.push("strarray".into());
                            }
                            JitKind::Number
                            | JitKind::Boolean
                            | JitKind::Dynamic
                            | JitKind::Dictionary => return None,
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Expr::Call(call)
                if array_predicate(call, parameters, locals, context.helpers).is_some() =>
            {
                let mut encoded = Vec::new();
                encode_expression(
                    array_predicate(call, parameters, locals, context.helpers)?,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                let kind = jit_expression_kind(&encoded)?.0;
                output.extend(encoded);
                output.push(
                    match kind {
                        JitKind::Array => "isarray",
                        JitKind::Dynamic => "dynisarray",
                        _ => "isnotarray",
                    }
                    .into(),
                );
            }
            Expr::Call(call)
                if array_method(call, parameters, locals, context.helpers).is_some() =>
            {
                let (method, receiver) = array_method(call, parameters, locals, context.helpers)?;
                let mut encoded = Vec::new();
                encode_expression(receiver, parameters, locals, context, &mut encoded)?;
                if encoded.len() == 1 {
                    if let Some(untag) = jit_typed_array_union_untag(&encoded[0]) {
                        encoded.push(untag.into());
                    } else if jit_dynamic_array_argument(&encoded[0]) {
                        encoded.push("untagarray".into());
                    }
                }
                if jit_expression_kind(&encoded)?.0 != JitKind::Array {
                    return None;
                }
                let dynamic_array = encoded.last().is_some_and(|token| token == "untagarray");
                if dynamic_array {
                    encoded.pop();
                }
                let prefix = array_prefix(&encoded)
                    .or_else(|| dynamic_array.then_some("dynamic"))?;
                if dynamic_array
                    && !matches!(
                        method,
                        "join"
                            | "toString"
                            | "slice"
                            | "concat"
                            | "at"
                            | "includes"
                            | "indexOf"
                            | "lastIndexOf"
                            | "toReversed"
                            | "reverse"
                            | "toSorted"
                            | "sort"
                            | "fill"
                            | "copyWithin"
                            | "with"
                            | "push"
                            | "unshift"
                            | "pop"
                            | "shift"
                            | "splice"
                            | "toSpliced"
                            | "some"
                            | "every"
                            | "find"
                            | "findIndex"
                            | "findLast"
                            | "findLastIndex"
                            | "filter"
                            | "map"
                            | "reduce"
                            | "reduceRight"
                    )
                {
                    return None;
                }
                let encoded_receiver = encoded.clone();
                let local_insert = matches!(method, "push" | "unshift")
                    .then(|| match receiver {
                        Expr::Ident(identifier) => locals
                            .get(identifier.sym.as_ref())
                            .and_then(|tokens| tokens.first())
                            .filter(|token| runtime_local_kind(token) == Some(JitKind::Array))
                            .and_then(|token| loop_local_index(token)),
                        _ => None,
                    })
                    .flatten()
                    .filter(|_| !call.args.is_empty());
                if local_insert.is_none() {
                    output.extend(encoded);
                }
                if matches!(method, "reduce" | "reduceRight") {
                    let (callback, initial) = match call.args.as_slice() {
                        [callback] => (callback, None),
                        [callback, initial] => (callback, Some(initial)),
                        _ => return None,
                    };
                    if prefix != "rn" && !dynamic_array {
                        return None;
                    }
                    if callback.spread.is_some()
                        || initial.is_some_and(|initial| initial.spread.is_some())
                    {
                        return None;
                    }
                    if let Some(initial) = initial {
                        encode_number(
                            initial.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                    } else {
                        output.push("c0000000000000000".into());
                    }
                    let reducer = (!dynamic_array).then(|| {
                        numeric_reducer(callback.expr.as_ref(), parameters, locals, context)
                    });
                    if let Some(operation) = reducer.flatten() {
                        output.push(format!(
                            "rnreduce{}{operation}{}",
                            if method == "reduceRight" { "right" } else { "" },
                            if initial.is_some() { "" } else { "0" }
                        ));
                    } else {
                        let (mut callback, mut kind, captures) = encode_numeric_jit_callback(
                            callback.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            4,
                            if dynamic_array && initial.is_none() {
                                None
                            } else {
                                Some(3)
                            },
                            ("a", "rn", dynamic_array),
                        )?;
                        if dynamic_array && initial.is_none() && kind != JitKind::Dynamic {
                            callback.push(
                                match kind {
                                    JitKind::Number => "tagnum",
                                    JitKind::Boolean => "tagbool",
                                    JitKind::String => "tagstr",
                                    _ => return None,
                                }
                                .into(),
                            );
                            kind = JitKind::Dynamic;
                        }
                        if kind
                            != if dynamic_array && initial.is_none() {
                                JitKind::Dynamic
                            } else {
                                JitKind::Number
                            }
                        {
                            return None;
                        }
                        encode_string(&callback.join(","), output)?;
                        let captured = !captures.is_empty();
                        if captured {
                            append_jit_captures(captures, output);
                        }
                        output.push(format!(
                            "{}reduce{}jit{}{}",
                            if dynamic_array { "dynarray" } else { "rn" },
                            if method == "reduceRight" { "right" } else { "" },
                            if captured { "c" } else { "" },
                            if initial.is_some() { "" } else { "0" }
                        ));
                    }
                } else if method == "map" {
                    let [callback] = call.args.as_slice() else {
                        return None;
                    };
                    if let Some(target) = primitive_conversion_map(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                    ) {
                        output.push(format!(
                            "{}mapto{target}",
                            if dynamic_array { "dynarray" } else { prefix }
                        ));
                    } else if prefix != "rn" {
                        if let Some(operation) = primitive_unary_map(
                            callback.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            prefix == "rb",
                        ) {
                            if dynamic_array && operation != "identity" {
                                return None;
                            }
                            output.push(if dynamic_array {
                                "dynarraymapidentity".into()
                            } else {
                                format!("{prefix}map{operation}")
                            });
                        } else {
                            let element_prefix = if prefix == "rb" { "b" } else { "s" };
                            let (callback, kind, captures) = encode_numeric_jit_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                3,
                                Some(2),
                                (element_prefix, prefix, dynamic_array),
                            )?;
                            let target = match kind {
                                JitKind::Number => "n",
                                JitKind::Boolean => "b",
                                JitKind::String => "s",
                                JitKind::Dynamic => return None,
                                JitKind::Array | JitKind::Dictionary => return None,
                            };
                            encode_string(&callback.join(","), output)?;
                            let captured = !captures.is_empty();
                            if captured {
                                append_jit_captures(captures, output);
                            }
                            output.push(format!(
                                "{}mapjit{target}{}",
                                if dynamic_array { "dynarray" } else { prefix },
                                if captured { "c" } else { "" }
                            ));
                        }
                    } else if let Some(operation) = encode_numeric_conditional_map(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        output,
                    ) {
                        output.push(operation);
                    } else if let Some(operation) = numeric_unary_map(
                        callback.expr.as_ref(), parameters, locals, context,
                    ) {
                        output.push(format!("rnmap{operation}"));
                    } else if let Some((operation, reverse)) = numeric_index_map(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                    ) {
                        output.push(format!(
                            "rnmapindex{}{operation}",
                            if reverse { "r" } else { "" }
                        ));
                    } else {
                        let mut operand = Vec::new();
                        if let Some((operation, reverse)) = encode_numeric_map_operand(
                            callback.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut operand,
                        ) {
                            output.extend(operand);
                            output.push(format!(
                                "rnmap{}{operation}",
                                if reverse { "r" } else { "" }
                            ));
                        } else {
                            let (callback, kind, captures) = encode_numeric_jit_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                3,
                                Some(2),
                                ("a", "rn", false),
                            )?;
                            if kind != JitKind::Number {
                                return None;
                            }
                            encode_string(&callback.join(","), output)?;
                            let captured = !captures.is_empty();
                            if captured {
                                append_jit_captures(captures, output);
                            }
                            output.push(if captured {
                                "rnmapjitc".into()
                            } else {
                                "rnmapjit".into()
                            });
                        }
                    }
                } else if matches!(
                    method,
                    "some"
                        | "every"
                        | "find"
                        | "findIndex"
                        | "findLast"
                        | "findLastIndex"
                        | "filter"
                ) {
                    let [callback] = call.args.as_slice() else {
                        return None;
                    };
                    let method = match method {
                        "findIndex" => "findindex",
                        "findLast" => "findlast",
                        "findLastIndex" => "findlastindex",
                        method => method,
                    };
                    if primitive_truthy_callback(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                    ) {
                        output.push(if dynamic_array {
                            format!("dynarray{method}truthy")
                        } else {
                            format!("{prefix}{method}truthy")
                        });
                    } else {
                        let mut operand = Vec::new();
                        let operation = if prefix == "rn" {
                            encode_numeric_quantifier_operand(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut operand,
                            )
                        } else {
                            encode_primitive_comparison_operand(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                match prefix {
                                    "rs" => JitKind::String,
                                    "rb" => JitKind::Boolean,
                                    "dynamic" => JitKind::Dynamic,
                                    _ => return None,
                                },
                                &mut operand,
                            )
                        };
                        if let Some(operation) = operation {
                            output.extend(operand);
                            output.push(format!(
                                "{}{method}{operation}",
                                if dynamic_array { "dynarray" } else { prefix }
                            ));
                        } else {
                            let element_prefix = match prefix {
                                "rn" => "a",
                                "rb" => "b",
                                "rs" => "s",
                                _ if dynamic_array => "u",
                                _ => return None,
                            };
                            let (callback, kind, captures) = encode_numeric_jit_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                3,
                                Some(2),
                                (element_prefix, prefix, dynamic_array),
                            )?;
                            if !matches!(kind, JitKind::Number | JitKind::Boolean) {
                                return None;
                            }
                            encode_string(&callback.join(","), output)?;
                            let captured = !captures.is_empty();
                            if captured {
                                append_jit_captures(captures, output);
                            }
                            output.push(format!(
                                "{}{method}jit{}",
                                if dynamic_array { "dynarray" } else { prefix },
                                if captured { "c" } else { "" }
                            ));
                        }
                    }
                } else if matches!(method, "join" | "toString") {
                    match call.args.as_slice() {
                        [] => encode_string(",", output)?,
                        [separator] if method == "join" => {
                            let mut encoded_separator = Vec::new();
                            encode_expression(
                                separator.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded_separator,
                            )?;
                            append_string(encoded_separator, output)?;
                        }
                        _ => return None,
                    }
                    output.push(if dynamic_array {
                        "dynarrayjoin".into()
                    } else {
                        format!("{prefix}join")
                    });
                } else if matches!(method, "push" | "unshift") {
                    if call.args.is_empty() {
                        output.push("arraylen".into());
                    }
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    let arguments: Vec<_> = if method == "unshift" {
                        call.args.iter().rev().collect()
                    } else {
                        call.args.iter().collect()
                    };
                    for (index, argument) in arguments.iter().enumerate() {
                        if index != 0 && local_insert.is_none() {
                            output.extend(encoded_receiver.iter().cloned());
                        }
                        let mut encoded_value = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded_value,
                        )?;
                        if jit_expression_kind(&encoded_value)?.0 != expected {
                            return None;
                        }
                        output.extend(encoded_value);
                        output.push(if dynamic_array {
                            format!("dynarray{method}")
                        } else {
                            local_insert.map_or_else(
                                || format!("{prefix}{method}"),
                                |local| format!("{prefix}l{method}{local}"),
                            )
                        });
                        if index + 1 != arguments.len() {
                            output.push("drop".into());
                        }
                    }
                } else if matches!(method, "pop" | "shift") {
                    if !call.args.is_empty() {
                        return None;
                    }
                    output.push(if dynamic_array {
                        format!("dynarray{method}")
                    } else {
                        format!("{prefix}{method}")
                    });
                } else if matches!(method, "splice" | "toSpliced") {
                    let scalar_kind = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    match call.args.as_slice() {
                        [] => {
                            output.push("c0000000000000000".into());
                            output.push("c0000000000000000".into());
                        }
                        [start] => {
                            encode_number(
                                start.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                            output.push("c7ff0000000000000".into());
                        }
                        [start, delete_count, ..] => {
                            encode_number(
                                start.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                            encode_number(
                                delete_count.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                        }
                    }
                    output.extend(encoded_receiver.iter().cloned());
                    output.push("c0000000000000000".into());
                    output.push("c0000000000000000".into());
                    output.push(if dynamic_array {
                        "dynarrayslice".into()
                    } else {
                        "arrayslice".into()
                    });
                    for argument in call.args.iter().skip(2) {
                        let mut encoded_value = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded_value,
                        )?;
                        if jit_expression_kind(&encoded_value)?.0 != scalar_kind {
                            return None;
                        }
                        output.extend(encoded_value);
                        output.push(if dynamic_array {
                            "dynarrayappend".into()
                        } else {
                            format!("{prefix}append")
                        });
                    }
                    output.push(
                        if dynamic_array {
                            if method == "splice" {
                                "dynarraysplice"
                            } else {
                                "dynarraytospliced"
                            }
                        } else if method == "splice" {
                            "arraysplice"
                        } else {
                            "arraytospliced"
                        }
                        .into(),
                    );
                } else if method == "concat" {
                    if call.args.is_empty() {
                        output.push("c0000000000000000".into());
                        output.push("c7ff0000000000000".into());
                        output.push(
                            if dynamic_array {
                                "dynarrayslice"
                            } else {
                                "arrayslice"
                            }
                            .into(),
                        );
                    }
                    if dynamic_array {
                        for argument in &call.args {
                            let mut encoded_argument = Vec::new();
                            encode_expression(
                                argument.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded_argument,
                            )?;
                            if encoded_argument.pop().as_deref() != Some("untagarray")
                                || jit_expression_kind(&encoded_argument)?.0 != JitKind::Dynamic
                            {
                                return None;
                            }
                            output.extend(encoded_argument);
                            output.push("dynarrayconcat".into());
                        }
                        return Some(());
                    }
                    let scalar_kind = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        _ => return None,
                    };
                    for argument in &call.args {
                        let mut encoded_argument = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded_argument,
                        )?;
                        match jit_expression_kind(&encoded_argument)?.0 {
                            JitKind::Array if array_prefix(&encoded_argument)? == prefix => {
                                output.extend(encoded_argument);
                                output.push("arrayconcat".into());
                            }
                            kind if kind == scalar_kind => {
                                output.extend(encoded_argument);
                                output.push(format!("{prefix}append"));
                            }
                            _ => return None,
                        }
                    }
                } else if matches!(method, "toReversed" | "reverse") {
                    if !call.args.is_empty() {
                        return None;
                    }
                    output.push(if method == "reverse" {
                        if dynamic_array {
                            "dynarrayreverse".into()
                        } else {
                            "arrayreverse".into()
                        }
                    } else if dynamic_array {
                        "dynarrayreversed".into()
                    } else {
                        "arrayreversed".into()
                    });
                } else if matches!(method, "toSorted" | "sort") {
                    let suffix = if method == "sort" { "sort" } else { "sorted" };
                    match call.args.as_slice() {
                        [] => output.push(if dynamic_array {
                            format!("dynarray{suffix}")
                        } else {
                            format!("{prefix}{suffix}")
                        }),
                        [callback] if callback.spread.is_none() && prefix == "rn" => output.push(
                            format!(
                                "rn{suffix}{}",
                                if numeric_sort_callback(
                                    callback.expr.as_ref(),
                                    parameters,
                                    locals,
                                    context,
                                )? {
                                    "desc"
                                } else {
                                    "asc"
                                }
                            ),
                        ),
                        [callback] if callback.spread.is_none() && prefix == "rs" => {
                            if string_sort_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                            )? {
                                output.push(format!("rs{suffix}desc"));
                            } else {
                                output.push(format!("rs{suffix}"));
                            }
                        }
                        _ => return None,
                    }
                } else if method == "fill" {
                    let [value, range @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    if range.len() > 2 {
                        return None;
                    }
                    let mut encoded = Vec::new();
                    encode_expression(value.expr.as_ref(), parameters, locals, context, &mut encoded)?;
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    if jit_expression_kind(&encoded)?.0 != expected {
                        return None;
                    }
                    output.extend(encoded);
                    match range {
                        [] => {
                            output.push("c0000000000000000".into());
                            output.push("c7ff0000000000000".into());
                        }
                        [start] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            output.push("c7ff0000000000000".into());
                        }
                        [start, end] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                        }
                        _ => unreachable!(),
                    }
                    output.push(if dynamic_array {
                        "dynarrayfill".into()
                    } else {
                        format!("{prefix}fill")
                    });
                } else if method == "copyWithin" {
                    match call.args.as_slice() {
                        [target, start] => {
                            encode_number(target.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            output.push("c7ff0000000000000".into());
                        }
                        [target, start, end] => {
                            encode_number(target.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                        }
                        _ => return None,
                    }
                    output.push(if dynamic_array {
                        "dynarraycopywithin".into()
                    } else {
                        "arraycopywithin".into()
                    });
                } else if method == "with" {
                    let [index, value] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(index.expr.as_ref(), parameters, locals, context, output)?;
                    let mut encoded = Vec::new();
                    encode_expression(value.expr.as_ref(), parameters, locals, context, &mut encoded)?;
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    if jit_expression_kind(&encoded)?.0 != expected {
                        return None;
                    }
                    output.extend(encoded);
                    output.push(if dynamic_array {
                        "dynarraywith".into()
                    } else {
                        format!("{prefix}with")
                    });
                } else if method == "slice" {
                    match call.args.as_slice() {
                        [] => {
                            output.push("c0000000000000000".into());
                            output.push("c7ff0000000000000".into());
                        }
                        [start] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            output.push("c7ff0000000000000".into());
                        }
                        [start, end] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                        }
                        _ => return None,
                    }
                    output.push(
                        if dynamic_array {
                            "dynarrayslice"
                        } else {
                            "arrayslice"
                        }
                        .into(),
                    );
                } else if method == "at" {
                    match call.args.as_slice() {
                        [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                        [index] => encode_number(
                            index.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        _ => return None,
                    }
                    output.push(if dynamic_array {
                        "dynarrayat".into()
                    } else {
                        format!("{prefix}at")
                    });
                } else {
                    let ([needle] | [needle, ..]) = call.args.as_slice() else {
                        return None;
                    };
                    let boolean_literal = matches!(needle.expr.as_ref(), Expr::Lit(Lit::Bool(_)));
                    let mut needle = {
                        let mut value = Vec::new();
                        encode_expression(
                            needle.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut value,
                        )?;
                        value
                    };
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rb" => JitKind::Boolean,
                        "rs" => JitKind::String,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    if expected == JitKind::Boolean && boolean_literal {
                        needle.push("asbool".into());
                    }
                    if jit_expression_kind(&needle)?.0 != expected {
                        return None;
                    }
                    output.append(&mut needle);
                    match call.args.get(1) {
                        Some(from) => encode_number(
                            from.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        None => output.push(format!(
                            "c{:016x}",
                            if method == "lastIndexOf" {
                                f64::INFINITY
                            } else {
                                0.0
                            }
                            .to_bits()
                        )),
                    }
                    if call.args.len() > 2 {
                        return None;
                    }
                    let operation = match method {
                        "includes" => "includes",
                        "indexOf" => "indexof",
                        "lastIndexOf" => "lastindexof",
                        _ => return None,
                    };
                    output.push(if dynamic_array {
                        format!("dynarray{operation}")
                    } else {
                        format!("{prefix}{operation}")
                    });
                }
            }
            Expr::Call(call) if primitive_value_of(call).is_some() => {
                encode_expression(
                    primitive_value_of(call)?,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
            }
            Expr::Call(call) if string_method(call, parameters, locals).is_some() => {
                let (operation, receiver) = string_method(call, parameters, locals)?;
                let mut encoded = Vec::new();
                encode_expression(receiver, parameters, locals, context, &mut encoded)?;
                match jit_expression_kind(&encoded)?.0 {
                    JitKind::String => output.extend(encoded),
                    JitKind::Dynamic => {
                        output.extend(encoded);
                        output.push("untagstr".into());
                    }
                    _ => return None,
                }
                if operation == "concat" {
                    for argument in &call.args {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_string(encoded, output)?;
                        output.push("concat".into());
                    }
                    return (output.len() <= 128).then_some(());
                } else if matches!(
                    operation,
                    "tolowercase"
                        | "touppercase"
                        | "iswellformed"
                        | "towellformed"
                        | "trim"
                        | "trimstart"
                        | "trimend"
                ) {
                    if !call.args.is_empty() {
                        return None;
                    }
                } else if operation == "normalize" {
                    match call.args.as_slice() {
                        [] => output.push("t4e4643".into()),
                        [form] => {
                            let mut encoded = Vec::new();
                            encode_expression(
                                form.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            append_string(encoded, output)?;
                        }
                        _ => return None,
                    }
                } else if operation == "repeat" {
                    let [count] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(count.expr.as_ref(), parameters, locals, context, output)?;
                } else if matches!(operation, "replace" | "replaceall") {
                    let [search, replacement] = call.args.as_slice() else {
                        return None;
                    };
                    for argument in [search, replacement] {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_string(encoded, output)?;
                    }
                } else if matches!(operation, "charat" | "charcodeat" | "at" | "codepointat") {
                    match call.args.as_slice() {
                        [] => output.push("c0000000000000000".into()),
                        [index] => {
                            encode_number(index.expr.as_ref(), parameters, locals, context, output)?
                        }
                        _ => return None,
                    }
                } else if matches!(operation, "padstart" | "padend") {
                    let [target, pad @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(target.expr.as_ref(), parameters, locals, context, output)?;
                    match pad {
                        [] => output.push("t20".into()),
                        [pad] => {
                            let mut encoded = Vec::new();
                            encode_expression(
                                pad.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            append_string(encoded, output)?;
                        }
                        _ => return None,
                    }
                } else if matches!(operation, "slice" | "substring") {
                    match call.args.as_slice() {
                        [] => output.push("c0000000000000000".into()),
                        [start] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?
                        }
                        [start, end] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                            output.push(format!("{operation}2"));
                            return Some(());
                        }
                        _ => return None,
                    }
                } else if operation == "split" {
                    let [separator, limit @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    let mut encoded = Vec::new();
                    encode_expression(
                        separator.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    append_string(encoded, output)?;
                    match limit {
                        [] => output.push("c7ff0000000000000".into()),
                        [limit] => encode_number(
                            limit.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        _ => return None,
                    }
                } else {
                    let [search, position @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    let mut encoded = Vec::new();
                    encode_expression(search.expr.as_ref(), parameters, locals, context, &mut encoded)?;
                    append_string(encoded, output)?;
                    match position {
                        [] => {}
                        [position] => {
                            encode_number(position.expr.as_ref(), parameters, locals, context, output)?;
                            output.push(format!("{operation}2"));
                            return Some(());
                        }
                        _ => return None,
                    }
                }
                output.push(operation.into());
            }
            Expr::Call(call) if number_parser(call, parameters, locals, context.helpers).is_some() => {
                let operation = number_parser(call, parameters, locals, context.helpers)?;
                let radix = if let Some((value, radix)) = call.args.split_first() {
                    let mut encoded = Vec::new();
                    encode_expression(
                        value.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    append_string(encoded, output)?;
                    radix
                } else {
                    encode_string("undefined", output)?;
                    &[]
                };
                if operation == "parsefloat" {
                    if !radix.is_empty() {
                        return None;
                    }
                } else {
                    match radix {
                        [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                        [radix] => encode_number(
                            radix.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        _ => return None,
                    }
                }
                output.push(operation.into());
            }
            Expr::Call(call)
                if number_predicate(call, parameters, locals, context.helpers).is_some() =>
            {
                let (operation, coercive) =
                    number_predicate(call, parameters, locals, context.helpers)?;
                let [argument] = call.args.as_slice() else {
                    return None;
                };
                let mut encoded = Vec::new();
                encode_expression(
                    argument.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                if coercive {
                    append_number(encoded, output)?;
                    output.push(operation.into());
                } else if jit_expression_kind(&encoded)?.0 == JitKind::Number {
                    output.extend(encoded);
                    output.push(operation.into());
                } else {
                    output.extend(encoded);
                    output.push(format!("c{:016x}", 0.0f64.to_bits()));
                    output.push("strictfalse".into());
                }
            }
            Expr::Call(call)
                if string_static_constructor(call, parameters, locals, context.helpers).is_some() =>
            {
                let operation = match string_static_constructor(
                    call,
                    parameters,
                    locals,
                    context.helpers,
                )? {
                    "fromCharCode" => "fromcharcode",
                    "fromCodePoint" => "fromcodepoint",
                    _ => unreachable!(),
                };
                if call.args.is_empty() {
                    encode_string("", output)?;
                }
                for (index, argument) in call.args.iter().enumerate() {
                    let mut encoded = Vec::new();
                    encode_expression(
                        argument.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    append_number(encoded, output)?;
                    output.push(operation.into());
                    if index != 0 {
                        output.push("concat".into());
                    }
                }
            }
            Expr::Call(call)
                if matches!(
                    &call.callee,
                    Callee::Expr(callee)
                        if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "String")
                ) =>
            {
                if parameters.contains_key("String") || locals.contains_key("String") {
                    return None;
                }
                match call.args.as_slice() {
                    [] => encode_string("", output)?,
                    [argument] if argument.spread.is_none() => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_string(encoded, output)?;
                    }
                    _ => return None,
                }
            }
            Expr::Call(call)
                if matches!(
                    &call.callee,
                    Callee::Expr(callee)
                        if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "Number")
                ) =>
            {
                if parameters.contains_key("Number") || locals.contains_key("Number") {
                    return None;
                }
                match call.args.as_slice() {
                    [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                    [argument] if argument.spread.is_none() => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_number(encoded, output)?;
                    }
                    _ => return None,
                }
            }
            Expr::Call(call)
                if matches!(
                    &call.callee,
                    Callee::Expr(callee)
                        if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "Boolean")
                ) =>
            {
                if parameters.contains_key("Boolean") || locals.contains_key("Boolean") {
                    return None;
                }
                match call.args.as_slice() {
                    [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                    [argument] if argument.spread.is_none() => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_boolean(encoded, output)?;
                    }
                    _ => return None,
                }
            }
            Expr::Call(call) if time_method(call, parameters, locals).is_some() => {
                let method = time_method(call, parameters, locals)?;
                if method == "processuptime" {
                    output.push("performancenow".into());
                    output.push(format!("c{:016x}", 1_000.0f64.to_bits()));
                    output.push("/".into());
                } else {
                    output.push(method.into());
                }
            }
            Expr::Call(call) if math_method(call, parameters, locals).is_some() => {
                let method = math_method(call, parameters, locals)?;
                if method == "random" {
                    if !call.args.is_empty() {
                        return None;
                    }
                    output.push("random".into());
                } else if matches!(
                    method,
                    "abs"
                        | "acos"
                        | "acosh"
                        | "asin"
                        | "asinh"
                        | "atan"
                        | "atanh"
                        | "cbrt"
                        | "ceil"
                        | "clz32"
                        | "cos"
                        | "cosh"
                        | "exp"
                        | "expm1"
                        | "floor"
                        | "fround"
                        | "log"
                        | "log1p"
                        | "log2"
                        | "log10"
                        | "round"
                        | "sign"
                        | "sin"
                        | "sinh"
                        | "sqrt"
                        | "tan"
                        | "tanh"
                        | "trunc"
                ) {
                    let [argument] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(argument.expr.as_ref(), parameters, locals, context, output)?;
                    output.push(method.into());
                } else if method == "pow" {
                    let [base, exponent] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(base.expr.as_ref(), parameters, locals, context, output)?;
                    encode_number(exponent.expr.as_ref(), parameters, locals, context, output)?;
                    output.push("pow".into());
                } else if matches!(method, "atan2" | "imul") {
                    let [y, x] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(y.expr.as_ref(), parameters, locals, context, output)?;
                    encode_number(x.expr.as_ref(), parameters, locals, context, output)?;
                    output.push(method.into());
                } else if call.args.is_empty() {
                    let value = match method {
                        "min" => f64::INFINITY,
                        "max" => f64::NEG_INFINITY,
                        "hypot" => 0.0,
                        _ => return None,
                    };
                    output.push(format!("c{:016x}", value.to_bits()));
                } else {
                    for (index, argument) in call.args.iter().enumerate() {
                        if argument.spread.is_some() {
                            let mut encoded = Vec::new();
                            encode_expression(
                                argument.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            if jit_expression_kind(&encoded)?.0 != JitKind::Array
                                || array_prefix(&encoded)? != "rn"
                            {
                                return None;
                            }
                            output.extend(encoded);
                            output.push(format!("rn{method}"));
                        } else {
                            encode_number(
                                argument.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                        }
                        if index != 0 {
                            output.push(method.into());
                        }
                    }
                    if method == "hypot"
                        && call.args.len() == 1
                        && call.args[0].spread.is_none()
                    {
                        output.push("abs".into());
                    }
                }
            }
            Expr::Call(call) => {
                encode_helper_call(call, parameters, locals, context, output)?;
            }
            Expr::Cond(conditional) => {
                encode_condition(conditional.test.as_ref(), parameters, locals, context, output)?;
                let narrowed = narrowed_locals(
                    conditional.test.as_ref(),
                    parameters,
                    locals,
                    context.helpers,
                );
                let consequent_locals = narrowed.as_ref().map_or(locals, |locals| &locals.0);
                let alternate_locals = narrowed.as_ref().map_or(locals, |locals| &locals.1);
                let mut consequent = Vec::new();
                let mut alternate = Vec::new();
                encode_expression(
                    conditional.cons.as_ref(),
                    parameters,
                    consequent_locals,
                    context,
                    &mut consequent,
                )?;
                encode_expression(
                    conditional.alt.as_ref(),
                    parameters,
                    alternate_locals,
                    context,
                    &mut alternate,
                )?;
                if boolean_literal(conditional.cons.as_ref()) {
                    consequent.push("asbool".into());
                }
                if boolean_literal(conditional.alt.as_ref()) {
                    alternate.push("asbool".into());
                }
                normalize_callable_branches([&mut consequent, &mut alternate])?;
                output.push("if".into());
                output.extend(consequent);
                output.push("else".into());
                output.extend(alternate);
                output.push("end".into());
            }
            _ => return None,
        }
        (output.len() <= 256).then_some(())
    }

    };
}
