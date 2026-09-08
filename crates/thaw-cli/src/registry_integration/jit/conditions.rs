macro_rules! jit_conditions {
    () => {
    fn encode_string(value: &str, output: &mut Vec<String>) -> Option<()> {
        if value.as_bytes().contains(&0) {
            return None;
        }
        output.push(format!(
            "t{}",
            value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        Some(())
    }


    fn encode_condition(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let Expr::Bin(binary) = expression {
            let operator = match binary.op {
                BinaryOp::Lt => Some("<"),
                BinaryOp::LtEq => Some("<="),
                BinaryOp::Gt => Some(">"),
                BinaryOp::GtEq => Some(">="),
                BinaryOp::EqEq | BinaryOp::EqEqEq => Some("=="),
                BinaryOp::NotEq | BinaryOp::NotEqEq => Some("!="),
                _ => None,
            };
            if let Some(operator) = operator {
                let mut left = Vec::new();
                let mut right = Vec::new();
                encode_expression(binary.left.as_ref(), parameters, locals, context, &mut left)?;
                encode_expression(binary.right.as_ref(), parameters, locals, context, &mut right)?;
                let left_kind = jit_expression_kind(&left)?.0;
                let right_kind = jit_expression_kind(&right)?.0;
                let strict = matches!(binary.op, BinaryOp::EqEqEq | BinaryOp::NotEqEq);
                if left_kind == JitKind::Dynamic || right_kind == JitKind::Dynamic {
                    append_dynamic(left, output)?;
                    append_dynamic(right, output)?;
                    output.push(
                        match binary.op {
                            BinaryOp::Lt => "dynlt",
                            BinaryOp::LtEq => "dynlte",
                            BinaryOp::Gt => "dyngt",
                            BinaryOp::GtEq => "dyngte",
                            BinaryOp::EqEq => "dyneq",
                            BinaryOp::NotEq => "dynne",
                            BinaryOp::EqEqEq => "dynseq",
                            BinaryOp::NotEqEq => "dynsne",
                            _ => unreachable!(),
                        }
                        .into(),
                    );
                } else if strict && left_kind != right_kind {
                    output.extend(left);
                    output.extend(right);
                    output.push(
                        if binary.op == BinaryOp::NotEqEq {
                            "stricttrue"
                        } else {
                            "strictfalse"
                        }
                        .into(),
                    );
                } else if left_kind == JitKind::String && right_kind == JitKind::String {
                    output.extend(left);
                    output.extend(right);
                    output.push("strcmp".into());
                    output.push(format!("c{:016x}", 0.0f64.to_bits()));
                    output.push(operator.into());
                } else {
                    append_number(left, output)?;
                    append_number(right, output)?;
                    output.push(operator.into());
                }
                return (output.len() <= 128).then_some(());
            }
        }
        let mut encoded = Vec::new();
        encode_expression(expression, parameters, locals, context, &mut encoded)?;
        append_boolean(encoded, output)?;
        (output.len() <= 256).then_some(())
    }

    enum NumericBody<'a> {
        Expression(&'a Expr),
        Statements(&'a [Stmt]),
    }

    fn boolean_literal(expression: &Expr) -> bool {
        let mut expression = expression;
        while let Expr::Paren(parenthesized) = expression {
            expression = parenthesized.expr.as_ref();
        }
        matches!(expression, Expr::Lit(Lit::Bool(_)))
    }

    fn dynamic_primitive_narrowing(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(String, JitKind, bool)> {
        let expression = match expression {
            Expr::Paren(expression) => expression.expr.as_ref(),
            expression => expression,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let equal = match binary.op {
            BinaryOp::EqEq | BinaryOp::EqEqEq => true,
            BinaryOp::NotEq | BinaryOp::NotEqEq => false,
            _ => return None,
        };
        let probe = |typeof_expression: &Expr, type_expression: &Expr| {
            let Expr::Unary(typeof_expression) = typeof_expression else {
                return None;
            };
            if typeof_expression.op != UnaryOp::TypeOf {
                return None;
            }
            let Expr::Ident(name) = typeof_expression.arg.as_ref() else {
                return None;
            };
            let Expr::Lit(Lit::Str(ty)) = type_expression else {
                return None;
            };
            let selected = match ty.value.to_string_lossy().as_ref() {
                "number" => JitKind::Number,
                "boolean" => JitKind::Boolean,
                "string" => JitKind::String,
                _ => return None,
            };
            (jit_expression_kind(locals.get(name.sym.as_ref())?)?.0 == JitKind::Dynamic)
                .then(|| (name.sym.to_string(), selected))
        };
        let (name, selected) = probe(binary.left.as_ref(), binary.right.as_ref())
            .or_else(|| probe(binary.right.as_ref(), binary.left.as_ref()))?;
        Some((name, selected, equal))
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_primitive_locals(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        let (name, selected, equal) = dynamic_primitive_narrowing(expression, locals)?;
        let source = locals.get(&name)?;
        let mut available = source
            .iter()
            .filter_map(|token| match token.as_str() {
                "tagnum" => Some(JitKind::Number),
                "tagbool" => Some(JitKind::Boolean),
                "tagstr" => Some(JitKind::String),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        for kind in source
            .iter()
            .find_map(|token| jit_dynamic_argument(token).map(|(_, kinds)| kinds))
            .into_iter()
            .flat_map(str::bytes)
        {
            let kind = match kind {
                b'n' => JitKind::Number,
                b'b' => JitKind::Boolean,
                b's' => JitKind::String,
                // Uppercase tags describe aggregate union members. They do
                // not participate in a primitive `typeof` narrowing, but
                // must not make the JIT extractor panic either.
                _ => continue,
            };
            available.insert(kind);
        }
        if available.is_empty() {
            available.extend(if source
                .iter()
                .any(|token| token.strip_prefix("ld").is_some())
            {
                [JitKind::Number, JitKind::Boolean, JitKind::String].as_slice()
            } else {
                [JitKind::Number, JitKind::String].as_slice()
            });
        }
        for token in source {
            available.remove(&match token.as_str() {
                "notnum" => JitKind::Number,
                "notbool" => JitKind::Boolean,
                "notstr" => JitKind::String,
                _ => continue,
            });
        }
        if !available.contains(&selected) {
            return None;
        }
        let narrow = |matches: bool| {
            let mut value = source.clone();
            let kind = if matches {
                selected
            } else {
                value.push(match selected {
                    JitKind::Number => "notnum",
                    JitKind::Boolean => "notbool",
                    JitKind::String => "notstr",
                    _ => return None,
                }
                .into());
                let remaining = available
                    .iter()
                    .copied()
                    .filter(|kind| *kind != selected)
                    .collect::<Vec<_>>();
                if remaining.len() != 1 {
                    let mut narrowed = locals.clone();
                    narrowed.insert(name.clone(), value);
                    return Some(narrowed);
                }
                remaining[0]
            };
            value.push(match kind {
                JitKind::Number => "untagnum",
                JitKind::Boolean => "untagbool",
                JitKind::String => "untagstr",
                _ => return None,
            }
            .into());
            let mut narrowed = locals.clone();
            narrowed.insert(name.clone(), value);
            Some(narrowed)
        };
        Some((narrow(equal)?, narrow(!equal)?))
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_array_locals(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        let expression = match expression {
            Expr::Paren(expression) => expression.expr.as_ref(),
            expression => expression,
        };
        let (call, positive) = match expression {
            Expr::Call(call) => (call, true),
            Expr::Unary(unary) if unary.op == UnaryOp::Bang => {
                let Expr::Call(call) = unary.arg.as_ref() else {
                    return None;
                };
                (call, false)
            }
            _ => return None,
        };
        let Expr::Ident(identifier) = array_predicate(call, parameters, locals, helpers)? else {
            return None;
        };
        let source = locals.get(identifier.sym.as_ref())?;
        let kinds = source
            .iter()
            .find_map(|token| jit_dynamic_argument(token).map(|(_, kinds)| kinds))?;
        let not_array = source.iter().any(|token| token == "notarray");
        let arrays = kinds
            .bytes()
            .filter(|kind| matches!(kind, b'N' | b'B' | b'S' | b'T' | b'X' | b'Y' | b'Z'))
            .collect::<Vec<_>>();
        if not_array || arrays.is_empty() {
            return None;
        }
        let mut array_value = source.clone();
        array_value.push(
            if arrays.iter().all(|kind| matches!(kind, b'N' | b'X')) {
                "untagarrayn"
            } else if arrays.iter().all(|kind| matches!(kind, b'B' | b'Y')) {
                "untagarrayb"
            } else if arrays.iter().all(|kind| matches!(kind, b'S' | b'Z')) {
                "untagarrays"
            } else {
                match arrays.as_slice() {
                [b'N'] => "untagrn",
                [b'B'] => "untagrb",
                [b'S'] => "untagrs",
                    [b'T' | b'X' | b'Y' | b'Z'] => "untagtuple",
                _ => "untagarray",
                }
            }
            .into(),
        );
        let remaining = kinds
            .bytes()
            .filter(|kind| !matches!(kind, b'N' | b'B' | b'S' | b'T' | b'X' | b'Y' | b'Z'))
            .collect::<Vec<_>>();
        let mut other_value = source.clone();
        if remaining.len() == 1 {
            other_value.push(
                match remaining[0] {
                    b'n' => "untagnum",
                    b'b' => "untagbool",
                    b's' => "untagstr",
                    b'D' => "untagdn",
                    b'E' => "untagdb",
                    b'F' => "untagds",
                    b'O' => "untagobject",
                    _ => return None,
                }
                .into(),
            );
        } else {
            other_value.push("notarray".into());
        }
        let narrowed = |value: Vec<String>| {
            let mut narrowed = locals.clone();
            narrowed.insert(identifier.sym.to_string(), value);
            narrowed
        };
        let branches = (narrowed(array_value), narrowed(other_value));
        Some(if positive {
            branches
        } else {
            (branches.1, branches.0)
        })
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_object_locals(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        let expression = match expression {
            Expr::Paren(expression) => expression.expr.as_ref(),
            expression => expression,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let equal = match binary.op {
            BinaryOp::EqEq | BinaryOp::EqEqEq => true,
            BinaryOp::NotEq | BinaryOp::NotEqEq => false,
            _ => return None,
        };
        let probe = |typeof_expression: &Expr, type_expression: &Expr| {
            let Expr::Unary(typeof_expression) = typeof_expression else {
                return None;
            };
            if typeof_expression.op != UnaryOp::TypeOf {
                return None;
            }
            let Expr::Ident(name) = typeof_expression.arg.as_ref() else {
                return None;
            };
            let Expr::Lit(Lit::Str(ty)) = type_expression else {
                return None;
            };
            (ty.value == *"object").then(|| name.sym.to_string())
        };
        let identifier = probe(binary.left.as_ref(), binary.right.as_ref())
            .or_else(|| probe(binary.right.as_ref(), binary.left.as_ref()))?;
        let source = locals.get(&identifier)?;
        let kinds = source
            .iter()
            .find_map(|token| jit_dynamic_argument(token).map(|(_, kinds)| kinds))?;
        let not_array = source.iter().any(|token| token == "notarray");
        let not_object = source.iter().any(|token| token == "notobject");
        let aggregates = kinds
            .bytes()
            .filter(|kind| matches!(kind, b'D' | b'E' | b'F' | b'O' | b'T' | b'X' | b'Y' | b'Z'))
            .collect::<Vec<_>>();
        if not_object
            || aggregates.is_empty()
            || (!not_array
                && kinds
                    .bytes()
                    .any(|kind| matches!(kind, b'N' | b'B' | b'S')))
        {
            return None;
        }
        let mut object_value = source.clone();
        object_value.push(
            match aggregates.as_slice() {
                [b'D'] => "untagdn",
                [b'E'] => "untagdb",
                [b'F'] => "untagds",
                [b'O'] => "untagobject",
                [b'T' | b'X' | b'Y' | b'Z'] => "untagtuple",
                _ if aggregates
                    .iter()
                    .all(|kind| matches!(kind, b'D' | b'E' | b'F')) =>
                {
                    "untagdictionary"
                }
                _ => return None,
            }
            .into(),
        );
        let remaining = kinds
            .bytes()
            .filter(|kind| {
                !(matches!(kind, b'D' | b'E' | b'F' | b'O' | b'T' | b'X' | b'Y' | b'Z')
                    || not_array && matches!(kind, b'N' | b'B' | b'S'))
            })
            .collect::<Vec<_>>();
        let mut other_value = source.clone();
        if remaining.len() == 1 {
            other_value.push(
                match remaining[0] {
                    b'n' => "untagnum",
                    b'b' => "untagbool",
                    b's' => "untagstr",
                    _ => return None,
                }
                .into(),
            );
        } else {
            other_value.push("notobject".into());
        }
        let narrowed = |value: Vec<String>| {
            let mut narrowed = locals.clone();
            narrowed.insert(identifier.clone(), value);
            narrowed
        };
        let branches = (narrowed(object_value), narrowed(other_value));
        Some(if equal {
            branches
        } else {
            (branches.1, branches.0)
        })
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_locals(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        narrowed_primitive_locals(expression, locals)
            .or_else(|| narrowed_array_locals(expression, parameters, locals, helpers))
            .or_else(|| narrowed_object_locals(expression, locals))
    }

    fn encode_switch_case_return(
        switch: &thaw_parser::ast::SwitchStmt,
        start: usize,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        switch.cases[start..]
            .iter()
            .find(|case| !case.cons.is_empty())
            .and_then(|case| {
                encode_returning_statements(&case.cons, parameters, locals, context, output)
            })
    }

    fn encode_returning_switch(
        switch: &thaw_parser::ast::SwitchStmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut discriminant = Vec::new();
        encode_expression(
            switch.discriminant.as_ref(),
            parameters,
            locals,
            context,
            &mut discriminant,
        )?;
        let kind = if boolean_literal(switch.discriminant.as_ref()) {
            JitKind::Boolean
        } else {
            jit_expression_kind(&discriminant)?.0
        };
        if matches!(kind, JitKind::Array | JitKind::Dictionary) {
            return None;
        }
        let default = switch.cases.iter().position(|case| case.test.is_none())?;
        let tested = switch
            .cases
            .iter()
            .enumerate()
            .filter_map(|(index, case)| case.test.as_ref().map(|_| index))
            .collect::<Vec<_>>();
        let mut returns = tested
            .iter()
            .copied()
            .chain(std::iter::once(default))
            .map(|index| {
                let mut encoded = Vec::new();
                encode_switch_case_return(
                    switch,
                    index,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                Some(encoded)
            })
            .collect::<Option<Vec<_>>>()?;
        normalize_callable_branches(returns.iter_mut())?;
        output.extend(discriminant);
        let mut branches = 0;
        for (return_index, index) in tested.iter().copied().enumerate() {
            let test = switch.cases[index].test.as_deref()?;
            output.push("dup".into());
            let mut encoded = Vec::new();
            encode_expression(test, parameters, locals, context, &mut encoded)?;
            let test_kind = if boolean_literal(test) {
                JitKind::Boolean
            } else {
                jit_expression_kind(&encoded)?.0
            };
            output.extend(encoded);
            if kind == JitKind::String && test_kind == JitKind::String {
                output.push("strcmp".into());
                output.push(format!("c{:016x}", 0.0f64.to_bits()));
                output.push("==".into());
            } else if kind == test_kind
                && matches!(kind, JitKind::Number | JitKind::Boolean)
            {
                output.push("==".into());
            } else {
                output.push("strictfalse".into());
            }
            output.push("if".into());
            output.extend(returns[return_index].iter().cloned());
            output.push("else".into());
            branches += 1;
        }
        output.extend(returns.last()?.iter().cloned());
        output.extend(std::iter::repeat_n("end".into(), branches));
        output.push("nip".into());
        (output.len() <= 128).then_some(())
    }

    fn encode_returning_statement(
        statement: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match statement {
            Stmt::Return(returned) => {
                let expression = returned.arg.as_deref()?;
                encode_expression(expression, parameters, locals, context, output)?;
                if boolean_literal(expression) {
                    output.push("asbool".into());
                }
                Some(())
            }
            Stmt::Block(block) => {
                encode_returning_statements(&block.stmts, parameters, locals, context, output)
            }
            Stmt::If(_) => encode_returning_statements(
                std::slice::from_ref(statement),
                parameters,
                locals,
                context,
                output,
            ),
            Stmt::Switch(switch) => {
                encode_returning_switch(switch, parameters, locals, context, output)
            }
            _ => None,
        }
    }

    fn encode_returning_statements(
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        if let Stmt::Return(_) = first {
            if !rest.is_empty() {
                return None;
            }
            return encode_returning_statement(first, parameters, locals, context, output);
        }
        if let Stmt::Switch(switch) = first {
            if !rest.is_empty() {
                return None;
            }
            return encode_returning_switch(switch, parameters, locals, context, output);
        }
        let Stmt::If(branch) = first else {
            return None;
        };
        encode_condition(branch.test.as_ref(), parameters, locals, context, output)?;
        let narrowed = narrowed_locals(
            branch.test.as_ref(),
            parameters,
            locals,
            context.helpers,
        );
        let consequent_locals = narrowed.as_ref().map_or(locals, |locals| &locals.0);
        let alternate_locals = narrowed.as_ref().map_or(locals, |locals| &locals.1);
        let mut consequent = Vec::new();
        encode_returning_statement(
            branch.cons.as_ref(),
            parameters,
            consequent_locals,
            context,
            &mut consequent,
        )?;
        let mut alternate_output = Vec::new();
        if let Some(alternate) = branch.alt.as_deref() {
            if !rest.is_empty() {
                return None;
            }
            encode_returning_statement(
                alternate,
                parameters,
                alternate_locals,
                context,
                &mut alternate_output,
            )?;
        } else {
            encode_returning_statements(
                rest,
                parameters,
                alternate_locals,
                context,
                &mut alternate_output,
            )?;
        }
        normalize_callable_branches([&mut consequent, &mut alternate_output])?;
        output.push("if".into());
        output.extend(consequent);
        output.push("else".into());
        output.extend(alternate_output);
        output.push("end".into());
        (output.len() <= 256).then_some(())
    }
    };
}
