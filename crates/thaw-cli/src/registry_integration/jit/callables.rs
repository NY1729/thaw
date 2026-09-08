macro_rules! jit_callables {
    () => {
    fn encode_numeric_body(
        body: NumericBody<'_>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match body {
            NumericBody::Expression(body) => {
                encode_expression(body, parameters, locals, context, output)?;
            }
            NumericBody::Statements(statements) => {
                encode_returning_statements(statements, parameters, locals, context, output)?;
            }
        }
        Some(())
    }


    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ExportStyle {
        Whole,
        Named,
    }

    #[derive(Clone, Copy)]
    enum NumericCallable<'a> {
        Function(&'a Function),
        Arrow(&'a ArrowExpr),
    }

    struct InlineContext<'a> {
        helpers: &'a std::collections::HashMap<String, NumericCallable<'a>>,
        module_locals: std::collections::HashMap<String, Vec<String>>,
        module_globals: std::collections::HashMap<String, u8>,
        callable_tables: std::collections::HashMap<String, Vec<(String, String)>>,
        callable_table_states:
            std::collections::HashMap<String, std::collections::HashMap<String, (u8, Vec<String>)>>,
        dynamic_callable_tables: std::collections::HashMap<String, (u8, Vec<String>)>,
        active: Vec<String>,
        recursive_names: Vec<String>,
        recursive_parameters: Vec<thaw_hir::HirType>,
        recursive_result: Option<JitKind>,
        loop_depth: usize,
        loop_labels: Vec<(String, usize)>,
    }

    fn same_callable(left: NumericCallable<'_>, right: NumericCallable<'_>) -> bool {
        match (left, right) {
            (NumericCallable::Function(left), NumericCallable::Function(right)) => {
                std::ptr::eq(left, right)
            }
            (NumericCallable::Arrow(left), NumericCallable::Arrow(right)) => {
                std::ptr::eq(left, right)
            }
            _ => false,
        }
    }

    fn callable_parts(callable: NumericCallable<'_>) -> Option<(Vec<&Pat>, Vec<LocalStep<'_>>, NumericBody<'_>)> {
        match callable {
            NumericCallable::Function(function)
                if !function.is_async && !function.is_generator =>
            {
                let body = function.body.as_ref()?;
                let (locals, body) = split_numeric_body(&body.stmts)?;
                Some((
                    function.params.iter().map(|parameter| &parameter.pat).collect(),
                    locals,
                    body,
                ))
            }
            NumericCallable::Arrow(function)
                if !function.is_async && !function.is_generator =>
            {
                let (locals, body) = match function.body.as_ref() {
                    thaw_parser::ast::ArrowFunctionBody::Expr(body) => {
                        (Vec::new(), NumericBody::Expression(body.as_ref()))
                    }
                    thaw_parser::ast::ArrowFunctionBody::FunctionBody(body) => {
                        split_numeric_body(&body.stmts)?
                    }
                };
                Some((function.params.iter().collect(), locals, body))
            }
            _ => None,
        }
    }

    fn numeric_reducer(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(accumulator), Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if accumulator.id.sym == value.id.sym {
            return None;
        }
        if !steps.is_empty() {
            let [LocalStep::Declare {
                name,
                mutable: false,
                ..
            }] = steps.as_slice()
            else {
                return None;
            };
            let NumericBody::Statements([Stmt::Return(statement)]) = &body else {
                return None;
            };
            if !matches!(statement.arg.as_deref(), Some(Expr::Ident(result)) if result.sym == name.sym)
            {
                return None;
            }
        }
        let callback_parameters = std::collections::HashMap::from([
            (accumulator.id.sym.to_string(), "a0".into()),
            (value.id.sym.to_string(), "a1".into()),
        ]);
        let mut encoded = Vec::new();
        encode_steps_and_body(
            steps,
            body,
            &callback_parameters,
            context.module_locals.clone(),
            context,
            &mut encoded,
        )?;
        let [left, right, operation] = encoded.as_slice() else {
            return None;
        };
        if left != "a0" || right != "a1" {
            return None;
        }
        match operation.as_str() {
            "+" => Some("add"),
            "-" => Some("sub"),
            "*" => Some("mul"),
            "/" => Some("div"),
            "%" => Some("rem"),
            "pow" => Some("pow"),
            "min" | "max"
                if !outer_parameters.contains_key("Math")
                    && !outer_locals.contains_key("Math")
                    && !context.helpers.contains_key("Math") =>
            {
                match operation.as_str() {
                    "min" => Some("min"),
                    "max" => Some("max"),
                    _ => unreachable!(),
                }
            }
            _ => None,
        }
    }

    fn numeric_sort_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<bool> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(left), Pat::Ident(right)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() || left.id.sym == right.id.sym {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        if binary.op != BinaryOp::Sub {
            return None;
        }
        if matches!(binary.left.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
        {
            Some(false)
        } else if matches!(binary.left.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
        {
            Some(true)
        } else {
            None
        }
    }

    fn string_sort_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<bool> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(left), Pat::Ident(right)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() || left.id.sym == right.id.sym {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Call(call) = expression else {
            return None;
        };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        let [argument] = call.args.as_slice() else {
            return None;
        };
        if argument.spread.is_some() || property.sym != "localeCompare" {
            return None;
        }
        if matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
        {
            Some(false)
        } else if matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
        {
            Some(true)
        } else {
            None
        }
    }

    fn encode_numeric_quantifier_operand(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let (operand, reverse) = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
        {
            (binary.right.as_ref(), false)
        } else if matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym) {
            (binary.left.as_ref(), true)
        } else {
            return None;
        };
        let operation = match (binary.op, reverse) {
            (BinaryOp::Lt, false) | (BinaryOp::Gt, true) => "lt",
            (BinaryOp::LtEq, false) | (BinaryOp::GtEq, true) => "lte",
            (BinaryOp::Gt, false) | (BinaryOp::Lt, true) => "gt",
            (BinaryOp::GtEq, false) | (BinaryOp::LtEq, true) => "gte",
            (BinaryOp::EqEq | BinaryOp::EqEqEq, _) => "eq",
            (BinaryOp::NotEq | BinaryOp::NotEqEq, _) => "ne",
            _ => return None,
        };
        let mut encoded = Vec::new();
        encode_expression(
            operand,
            outer_parameters,
            outer_locals,
            context,
            &mut encoded,
        )?;
        if encoded
            .iter()
            .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
        {
            return None;
        }
        if matches!(binary.op, BinaryOp::EqEqEq | BinaryOp::NotEqEq)
            && jit_expression_kind(&encoded)?.0 != JitKind::Number
        {
            return None;
        }
        append_number(encoded, output)?;
        Some(operation)
    }

    fn encode_primitive_comparison_operand(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        expected: JitKind,
        output: &mut Vec<String>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let (operand, reverse) = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
        {
            (binary.right.as_ref(), false)
        } else if matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym) {
            (binary.left.as_ref(), true)
        } else {
            return None;
        };
        let operation = match (binary.op, reverse) {
            (BinaryOp::Lt, false) | (BinaryOp::Gt, true) => "lt",
            (BinaryOp::LtEq, false) | (BinaryOp::GtEq, true) => "lte",
            (BinaryOp::Gt, false) | (BinaryOp::Lt, true) => "gt",
            (BinaryOp::GtEq, false) | (BinaryOp::LtEq, true) => "gte",
            (BinaryOp::EqEq, _) => "eq",
            (BinaryOp::NotEq, _) => "ne",
            (BinaryOp::EqEqEq, _) if expected == JitKind::Dynamic => "seq",
            (BinaryOp::NotEqEq, _) if expected == JitKind::Dynamic => "sne",
            (BinaryOp::EqEqEq, _) => "eq",
            (BinaryOp::NotEqEq, _) => "ne",
            _ => return None,
        };
        let mut encoded = Vec::new();
        encode_expression(operand, outer_parameters, outer_locals, context, &mut encoded)?;
        if jit_expression_kind(&encoded)?.0 != expected {
            return None;
        }
        output.extend(encoded);
        Some(operation)
    }

    fn primitive_truthy_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> bool {
        let boolean_is_shadowed = outer_parameters.contains_key("Boolean")
            || outer_locals.contains_key("Boolean")
            || context.module_locals.contains_key("Boolean")
            || context.helpers.contains_key("Boolean");
        if matches!(expression, Expr::Ident(identifier) if identifier.sym == "Boolean") {
            return !boolean_is_shadowed;
        }
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return false;
        }
        let Some(callable) = resolve_callable(expression, context.helpers) else {
            return false;
        };
        let Some((parameters, steps, body)) = callable_parts(callable) else {
            return false;
        };
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return false;
        };
        if !steps.is_empty() {
            return false;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => {
                let Some(expression) = statement.arg.as_deref() else {
                    return false;
                };
                expression
            }
            _ => return false,
        };
        if matches!(expression, Expr::Ident(identifier) if identifier.sym == value.id.sym) {
            return true;
        }
        if matches!(expression, Expr::Unary(outer)
            if outer.op == UnaryOp::Bang
                && matches!(outer.arg.as_ref(), Expr::Unary(inner)
                    if inner.op == UnaryOp::Bang
                        && matches!(inner.arg.as_ref(), Expr::Ident(identifier)
                            if identifier.sym == value.id.sym)))
        {
            return true;
        }
        let Expr::Call(call) = expression else {
            return false;
        };
        let Callee::Expr(callee) = &call.callee else {
            return false;
        };
        let [argument] = call.args.as_slice() else {
            return false;
        };
        !boolean_is_shadowed
            && value.id.sym != "Boolean"
            && matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "Boolean")
            && argument.spread.is_none()
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym)
    }

    fn encode_numeric_map_operand(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<(&'static str, bool)> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        let marker = (0..16)
            .map(|index| format!("a{index}"))
            .find(|candidate| {
                !outer_parameters.values().any(|token| token == candidate)
                    && !outer_locals
                        .values()
                        .flatten()
                        .any(|token| token == candidate)
            })?;
        let mut callback_parameters = outer_parameters.clone();
        callback_parameters.insert(value.id.sym.to_string(), marker.clone());
        let mut encoded = Vec::new();
        encode_steps_and_body(
            steps,
            body,
            &callback_parameters,
            outer_locals.clone(),
            context,
            &mut encoded,
        )?;
        if encoded.iter().filter(|token| **token == marker).count() != 1 {
            return None;
        }
        let operation = match encoded.pop()?.as_str() {
            "+" => "add",
            "-" => "sub",
            "*" => "mul",
            "/" => "div",
            "%" => "rem",
            "pow" => "pow",
            _ => return None,
        };
        let reverse = if encoded.first() == Some(&marker) {
            encoded.remove(0);
            false
        } else if encoded.last() == Some(&marker) {
            encoded.pop();
            true
        } else {
            return None;
        };
        if encoded.is_empty()
            || jit_expression_kind(&encoded)?.0 != JitKind::Number
            || encoded
                .iter()
                .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
        {
            return None;
        }
        output.extend(encoded);
        Some((operation, reverse))
    }

    fn encode_numeric_conditional_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<String> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Cond(conditional) = expression else {
            return None;
        };
        let Expr::Bin(comparison) = conditional.test.as_ref() else {
            return None;
        };
        let (operand, reverse) = if matches!(comparison.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
        {
            (comparison.right.as_ref(), false)
        } else if matches!(comparison.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym)
        {
            (comparison.left.as_ref(), true)
        } else {
            return None;
        };
        let operation = match comparison.op {
            BinaryOp::Lt => "lt",
            BinaryOp::LtEq => "lte",
            BinaryOp::Gt => "gt",
            BinaryOp::GtEq => "gte",
            BinaryOp::EqEq | BinaryOp::EqEqEq => "eq",
            BinaryOp::NotEq | BinaryOp::NotEqEq => "ne",
            _ => return None,
        };
        let mut encoded = Vec::new();
        encode_expression(
            operand,
            outer_parameters,
            outer_locals,
            context,
            &mut encoded,
        )?;
        if jit_expression_kind(&encoded)?.0 != JitKind::Number
            || encoded
                .iter()
                .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
        {
            return None;
        }
        let branch_mode = |branch: &Expr, context: &mut InlineContext<'_>| {
            if matches!(branch, Expr::Ident(identifier) if identifier.sym == value.id.sym) {
                return Some(0u8);
            }
            let mut branch_encoded = Vec::new();
            if encode_expression(
                branch,
                outer_parameters,
                outer_locals,
                context,
                &mut branch_encoded,
            )
            .is_some()
                && branch_encoded == encoded
            {
                return Some(1);
            }
            let Expr::Bin(binary) = branch else {
                return None;
            };
            let (other, reverse) = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
            {
                (binary.right.as_ref(), false)
            } else if matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym)
            {
                (binary.left.as_ref(), true)
            } else {
                return None;
            };
            let mut other_encoded = Vec::new();
            encode_expression(
                other,
                outer_parameters,
                outer_locals,
                context,
                &mut other_encoded,
            )?;
            if other_encoded != encoded {
                return None;
            }
            let operation = match binary.op {
                BinaryOp::Add => 0,
                BinaryOp::Sub => 1,
                BinaryOp::Mul => 2,
                BinaryOp::Div => 3,
                BinaryOp::Mod => 4,
                BinaryOp::Exp => 5,
                _ => return None,
            };
            Some(if reverse { 8 } else { 2 } + operation)
        };
        let true_branch = branch_mode(conditional.cons.as_ref(), context)?;
        let false_branch = branch_mode(conditional.alt.as_ref(), context)?;
        if true_branch == false_branch {
            return None;
        }
        output.extend(encoded);
        if matches!((true_branch, false_branch), (0, 1) | (1, 0)) {
            return Some(format!(
                "rnmapselect{operation}{}",
                u8::from(reverse) | if true_branch == 0 { 0 } else { 2 }
            ));
        }
        let comparison = match operation {
            "lt" => 0,
            "lte" => 1,
            "gt" => 2,
            "gte" => 3,
            "eq" => 4,
            "ne" => 5,
            _ => unreachable!(),
        };
        let encoded = comparison
            | (u16::from(reverse) << 3)
            | (u16::from(true_branch) << 4)
            | (u16::from(false_branch) << 8);
        Some(format!("rnmapbranch{encoded:03x}"))
    }

    fn numeric_index_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<(&'static str, bool)> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value), Pat::Ident(index)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let reverse = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == index.id.sym)
        {
            false
        } else if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == index.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym)
        {
            true
        } else {
            return None;
        };
        Some((
            match binary.op {
                BinaryOp::Add => "add",
                BinaryOp::Sub => "sub",
                BinaryOp::Mul => "mul",
                BinaryOp::Div => "div",
                BinaryOp::Mod => "rem",
                BinaryOp::Exp => "pow",
                _ => return None,
            },
            reverse,
        ))
    }

    fn encode_numeric_jit_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        max_parameters: usize,
        array_parameter: Option<usize>,
        callback_abi: (&str, &str, bool),
    ) -> Option<(Vec<String>, JitKind, Vec<Vec<String>>)> {
        let (element_prefix, array_parameter_prefix, dynamic_array) = callback_abi;
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        if parameters.len() > max_parameters {
            return None;
        }
        let mut callback_parameters = std::collections::HashMap::new();
        for (index, parameter) in parameters.iter().enumerate() {
            let Pat::Ident(parameter) = parameter else {
                return None;
            };
            if callback_parameters
                .insert(
                    parameter.id.sym.to_string(),
                    if dynamic_array {
                        match (max_parameters, array_parameter, index) {
                            (3, _, 0) => "u0nbs".into(),
                            (3, _, 1) => "a2".into(),
                            (3, _, 2) => "u3NBS".into(),
                            (4, Some(3), 0) => "a0".into(),
                            (4, Some(3), 1) => "u1nbs".into(),
                            (4, Some(3), 2) => "a3".into(),
                            (4, Some(3), 3) => "u4NBS".into(),
                            (4, None, 0) => "u0nbs".into(),
                            (4, None, 1) => "u2nbs".into(),
                            (4, None, 2) => "a4".into(),
                            (4, None, 3) => "u5NBS".into(),
                            _ => return None,
                        }
                    } else {
                        format!(
                            "{}{}",
                            if array_parameter == Some(index) {
                                array_parameter_prefix
                            } else if index == 0 {
                                element_prefix
                            } else {
                                "a"
                            },
                            index
                        )
                    },
                )
                .is_some()
            {
                return None;
            }
        }
        let mut captures = outer_parameters
            .iter()
            .filter(|(name, _)| !callback_parameters.contains_key(name.as_str()))
            .map(|(name, token)| (name.clone(), vec![token.clone()]))
            .chain(
                outer_locals
                    .iter()
                    .filter(|(name, _)| !callback_parameters.contains_key(name.as_str()))
                    .map(|(name, tokens)| (name.clone(), tokens.clone())),
            )
            .collect::<Vec<_>>();
        captures.sort_by(|left, right| left.0.cmp(&right.0));
        if dynamic_array {
            captures.retain(|(_, tokens)| {
                jit_expression_kind(tokens).is_some_and(|(kind, _)| {
                    kind != JitKind::Dynamic
                        && (kind != JitKind::Array || array_prefix(tokens).is_some())
                })
            });
        }
        let capture_offset = if dynamic_array {
            max_parameters + if array_parameter.is_some() { 2 } else { 3 }
        } else {
            max_parameters
        };
        if capture_offset + captures.len() > 16 {
            return None;
        }
        let mut callback_locals = context.module_locals.clone();
        let mut capture_tokens = Vec::new();
        for (offset, (name, tokens)) in captures.iter().enumerate() {
            let prefix = match jit_expression_kind(tokens)?.0 {
                JitKind::Number => "a",
                JitKind::Boolean => "b",
                JitKind::String => "s",
                JitKind::Dynamic => return None,
                JitKind::Array => array_prefix(tokens)?,
                JitKind::Dictionary => dictionary_prefix(tokens)?,
            };
            let token = format!("{prefix}{}", capture_offset + offset);
            capture_tokens.push(token.clone());
            if outer_parameters.contains_key(name) {
                callback_parameters.insert(name.clone(), token);
            } else {
                callback_locals.insert(name.clone(), vec![token]);
            }
        }
        let mut encoded = Vec::new();
        encode_steps_and_body(
            steps,
            body,
            &callback_parameters,
            callback_locals,
            context,
            &mut encoded,
        )?;
        let mut selected_captures = Vec::new();
        for ((_, capture), token) in captures.into_iter().zip(capture_tokens) {
            if encoded.iter().any(|encoded| encoded == &token) {
                let prefix = token.trim_end_matches(|character: char| character.is_ascii_digit());
                let replacement = format!("{prefix}{}", capture_offset + selected_captures.len());
                for encoded in &mut encoded {
                    if encoded == &token {
                        *encoded = replacement.clone();
                    }
                }
                selected_captures.push(capture);
            }
        }
        let kind = jit_expression_kind(&encoded)?.0;
        stable_jit_tokens(&encoded).then_some((encoded, kind, selected_captures))
    }

    fn append_jit_captures(captures: Vec<Vec<String>>, output: &mut Vec<String>) {
        output.push("arrayempty".into());
        for capture in captures {
            output.extend(capture);
            output.push("captureappend".into());
        }
    }

    fn stable_jit_tokens(tokens: &[String]) -> bool {
        !tokens
            .iter()
            .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
    }

    fn numeric_unary_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        if matches!(expression, Expr::Unary(unary)
            if unary.op == UnaryOp::Minus
                && matches!(unary.arg.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        {
            return Some("neg");
        }
        if value.id.sym == "Math"
            || context.helpers.contains_key("Math")
            || context.module_locals.contains_key("Math")
        {
            return None;
        }
        let Expr::Call(call) = expression else {
            return None;
        };
        let [argument] = call.args.as_slice() else {
            return None;
        };
        let operation = math_method(call, outer_parameters, outer_locals)?;
        (argument.spread.is_none()
            && is_unary_math_method(operation)
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        .then_some(operation)
    }

    fn primitive_unary_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
        boolean: bool,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        if matches!(expression, Expr::Ident(identifier) if identifier.sym == value.id.sym) {
            return Some("identity");
        }
        if boolean
            && matches!(expression, Expr::Unary(unary)
                if unary.op == UnaryOp::Bang
                    && matches!(unary.arg.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        {
            return Some("not");
        }
        if !boolean
            && matches!(expression, Expr::Member(member)
                if matches!(&member.prop, MemberProp::Ident(property) if property.sym == "length")
                    && matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        {
            return Some("length");
        }
        let Expr::Call(call) = expression else {
            return None;
        };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        if boolean
            || !call.args.is_empty()
            || !matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym)
        {
            return None;
        }
        match property.sym.as_ref() {
            "toLowerCase" => Some("tolowercase"),
            "toUpperCase" => Some("touppercase"),
            "trim" => Some("trim"),
            "trimStart" => Some("trimstart"),
            "trimEnd" => Some("trimend"),
            _ => None,
        }
    }

    fn primitive_conversion_map(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<&'static str> {
        let Expr::Ident(identifier) = expression else {
            return None;
        };
        let name = identifier.sym.as_ref();
        if parameters.contains_key(name)
            || locals.contains_key(name)
            || context.module_locals.contains_key(name)
            || context.helpers.contains_key(name)
        {
            return None;
        }
        match name {
            "Number" => Some("number"),
            "Boolean" => Some("boolean"),
            "String" => Some("string"),
            _ => None,
        }
    }

    const CALLABLE_ALIAS_PREFIX: &str = "\0call:";
    const CALLABLE_LOCAL_PREFIX: &str = "\0calllocal:";
    const CALLABLE_MEMBER_PREFIX: &str = "\0callmember:";

    fn callable_local_alias(tokens: &[String]) -> Option<(String, Vec<String>)> {
        let [marker] = tokens else {
            return None;
        };
        let (local, helpers) = marker
            .strip_prefix(CALLABLE_LOCAL_PREFIX)?
            .split_once(':')?;
        Some((
            local.to_owned(),
            helpers.split('|').map(str::to_owned).collect(),
        ))
    }

    fn callable_member_alias(tokens: &[String]) -> Option<(String, Vec<String>)> {
        let [marker] = tokens else {
            return None;
        };
        let (table, key) = marker
            .strip_prefix(CALLABLE_MEMBER_PREFIX)?
            .split_once(':')?;
        Some((
            table.to_owned(),
            key.split(';').map(str::to_owned).collect(),
        ))
    }

    fn encode_callable_member_snapshot(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        runtime_locals: &mut usize,
        output: &mut Vec<String>,
    ) -> Option<String> {
        let expression = match expression {
            Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
            expression => expression,
        };
        let Expr::Member(member) = expression else {
            return None;
        };
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        let static_key = match &member.prop {
            MemberProp::Ident(property) => Some(property.sym.to_string()),
            MemberProp::Computed(property) => {
                if let Expr::Lit(Lit::Str(value)) = property.expr.as_ref() {
                    Some(value.value.to_string_lossy().into_owned())
                } else {
                    None
                }
            }
            MemberProp::PrivateName(_) => return None,
        };
        if let Some((table_id, helpers)) = context
            .dynamic_callable_tables
            .get(table.sym.as_ref())
            .cloned()
        {
            let local = *runtime_locals;
            *runtime_locals += 1;
            if let Some(key) = static_key.as_deref() {
                encode_string(key, output)?;
            } else {
                let MemberProp::Computed(property) = &member.prop else {
                    unreachable!()
                };
                let mut key = Vec::new();
                encode_expression(
                    property.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut key,
                )?;
                append_string(key, output)?;
            }
            let initial = context
                .callable_tables
                .get(table.sym.as_ref())?
                .iter()
                .map(|(key, helper)| {
                    let selection = helpers
                        .iter()
                        .position(|candidate| candidate == helper)?;
                    Some((
                        key.clone(),
                        vec![format!("c{:016x}", (selection as f64).to_bits())],
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            output.extend([
                "dup".into(),
                format!("c{:016x}", (table_id as f64).to_bits()),
                "callableget".into(),
                "dup".into(),
                format!("c{:016x}", (-1.0f64).to_bits()),
                "!=".into(),
                "if".into(),
                "dup".into(),
                "else".into(),
                "dup2".into(),
                "drop".into(),
            ]);
            encode_callable_table_branches(
                &initial,
                &format!("c{:016x}", (-1.0f64).to_bits()),
                output,
            )?;
            output.extend([
                "nip".into(),
                "end".into(),
                "nip".into(),
                "nip".into(),
            ]);
            return Some(format!(
                "{CALLABLE_LOCAL_PREFIX}ln{local}:{}",
                helpers.join("|")
            ));
        }
        let key = static_key?;
        let (slot, helpers) = context
            .callable_table_states
            .get(table.sym.as_ref())?
            .get(&key)?;
        let local = *runtime_locals;
        *runtime_locals += 1;
        output.extend([
            format!("c{:016x}", (*slot as f64).to_bits()),
            "globalget".into(),
        ]);
        Some(format!(
            "{CALLABLE_LOCAL_PREFIX}ln{local}:{}",
            helpers.join("|")
        ))
    }

    fn encode_callable_member_alias(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<String> {
        if let Expr::Paren(parenthesized) = expression {
            return encode_callable_member_alias(
                parenthesized.expr.as_ref(),
                parameters,
                locals,
                context,
            );
        }
        let Expr::Member(member) = expression else {
            return None;
        };
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        context.callable_tables.get(table.sym.as_ref())?;
        // Re-dispatching through a mutable table at call time would not preserve
        // JavaScript's get-time function identity. Those aliases need a runtime
        // selector snapshot and deliberately remain on the fallback path here.
        if context
            .dynamic_callable_tables
            .contains_key(table.sym.as_ref())
            || context.callable_table_states.contains_key(table.sym.as_ref())
        {
            return None;
        }
        let key = match &member.prop {
            MemberProp::Ident(property) => {
                let mut encoded = Vec::new();
                encode_string(property.sym.as_ref(), &mut encoded)?;
                encoded
            }
            MemberProp::Computed(property) => {
                let mut encoded = Vec::new();
                encode_expression(
                    property.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                encoded
            }
            MemberProp::PrivateName(_) => return None,
        };
        if !matches!(
            jit_expression_kind(&key)?.0,
            JitKind::Number | JitKind::Boolean | JitKind::String
        ) || !stable_jit_tokens(&key)
        {
            return None;
        }
        Some(format!(
            "{CALLABLE_MEMBER_PREFIX}{}:{}",
            table.sym,
            key.join(";")
        ))
    }

    fn callable_alias(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<String> {
        match expression {
            Expr::Ident(identifier) => {
                if let Some(local) = locals.get(identifier.sym.as_ref()) {
                    let [marker] = local.as_slice() else {
                        return None;
                    };
                    marker.strip_prefix(CALLABLE_ALIAS_PREFIX).map(str::to_owned)
                } else {
                    context
                        .helpers
                        .contains_key(identifier.sym.as_ref())
                        .then(|| identifier.sym.to_string())
                }
            }
            Expr::Paren(parenthesized) => callable_alias(parenthesized.expr.as_ref(), locals, context),
            _ => None,
        }
    }

    fn encode_helper_target(
        callee: &Expr,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let Expr::Paren(parenthesized) = callee {
            return encode_helper_target(
                parenthesized.expr.as_ref(),
                arguments,
                parameters,
                locals,
                context,
                output,
            );
        }
        if let Expr::Cond(conditional) = callee {
            encode_condition(
                conditional.test.as_ref(),
                parameters,
                locals,
                context,
                output,
            )?;
            output.push("if".into());
            encode_helper_target(
                conditional.cons.as_ref(),
                arguments,
                parameters,
                locals,
                context,
                output,
            )?;
            output.push("else".into());
            encode_helper_target(
                conditional.alt.as_ref(),
                arguments,
                parameters,
                locals,
                context,
                output,
            )?;
            output.push("end".into());
            return Some(());
        }
        if let Expr::Member(member) = callee {
            return encode_callable_table_call(
                member, arguments, parameters, locals, context, output,
            );
        }
        if let Expr::Ident(identifier) = callee {
            if let Some((table, key)) = locals
                .get(identifier.sym.as_ref())
                .and_then(|tokens| callable_member_alias(tokens))
            {
                return encode_callable_table_key_call(
                    &table,
                    key,
                    arguments,
                    parameters,
                    locals,
                    context,
                    output,
                );
            }
            if let Some((local, helpers)) = locals
                .get(identifier.sym.as_ref())
                .and_then(|tokens| callable_local_alias(tokens))
            {
                let mut branches = Vec::with_capacity(helpers.len());
                for helper in helpers {
                    let mut encoded = Vec::new();
                    encode_named_helper_target(
                        &helper,
                        arguments,
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    branches.push(encoded);
                }
                let kind = normalize_callable_branches(&mut branches)?;
                return encode_local_callable_branches(
                    &local,
                    &branches,
                    0,
                    missing_callable(kind),
                    output,
                );
            }
        }
        let name = callable_alias(callee, locals, context)?;
        encode_named_helper_target(
            name.as_str(),
            arguments,
            parameters,
            locals,
            context,
            output,
        )
    }

    fn encode_local_callable_branches(
        local: &str,
        branches: &[Vec<String>],
        selection: usize,
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((branch, remaining)) = branches.split_first() else {
            output.push(missing.into());
            return Some(());
        };
        output.extend([
            local.into(),
            format!("c{:016x}", (selection as f64).to_bits()),
            "==".into(),
            "if".into(),
        ]);
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_local_callable_branches(local, remaining, selection + 1, missing, output)?;
        output.push("end".into());
        Some(())
    }

    fn encode_named_helper_target(
        name: &str,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if context.recursive_names.iter().any(|recursive| recursive == name) {
            if arguments.len() != context.recursive_parameters.len()
                || arguments.iter().any(|argument| argument.spread.is_some())
            {
                return None;
            }
            let expected_parameters = context.recursive_parameters.clone();
            let mut arity = 0;
            for (argument, expected) in arguments.iter().zip(&expected_parameters) {
                arity += encode_recursive_argument(
                    argument.expr.as_ref(),
                    expected,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
            }
            let result = match context.recursive_result? {
                JitKind::Number => 'n',
                JitKind::Boolean => 'b',
                JitKind::String => 's',
                JitKind::Dynamic => return None,
                JitKind::Array => return None,
                JitKind::Dictionary => return None,
            };
            output.push(format!("recur{result}{arity}"));
            return Some(());
        }
        if parameters.contains_key(name)
            || locals.contains_key(name)
            || context.active.len() >= 16
            || context.active.iter().any(|active| active == name)
        {
            return None;
        }
        let callable = *context.helpers.get(name)?;
        let (helper_parameters, steps, body) = callable_parts(callable)?;
        if helper_parameters.len() != arguments.len()
            || arguments.iter().any(|argument| argument.spread.is_some())
        {
            return None;
        }
        let mut helper_locals = context.module_locals.clone();
        let mut bound_parameters = std::collections::HashSet::new();
        for (parameter, argument) in helper_parameters.iter().zip(arguments) {
            let Pat::Ident(parameter) = parameter else {
                return None;
            };
            if !bound_parameters.insert(parameter.id.sym.to_string()) {
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
            helper_locals.insert(parameter.id.sym.to_string(), encoded);
        }
        context.active.push(name.to_string());
        let result = encode_steps_and_body(
            steps,
            body,
            &std::collections::HashMap::new(),
            helper_locals,
            context,
            output,
        );
        context.active.pop();
        result
    }

    fn encode_callable_table_call(
        member: &thaw_parser::ast::MemberExpr,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        let entries = context.callable_tables.get(table.sym.as_ref())?.clone();
        match &member.prop {
            MemberProp::Ident(property) => {
                let (_, helper) = entries
                    .iter()
                    .find(|(key, _)| key == property.sym.as_ref())?;
                if context
                    .dynamic_callable_tables
                    .contains_key(table.sym.as_ref())
                {
                    let mut branch = Vec::new();
                    encode_named_helper_target(
                        helper,
                        arguments,
                        parameters,
                        locals,
                        context,
                        &mut branch,
                    )?;
                    let kind = jit_expression_kind(&branch)?.0;
                    let missing = missing_callable(kind);
                    encode_string(property.sym.as_ref(), output)?;
                    return encode_dynamic_callable_table_dispatch(
                        table.sym.as_ref(),
                        &[(property.sym.to_string(), branch)],
                        arguments,
                        parameters,
                        locals,
                        context,
                        kind,
                        missing,
                        output,
                    );
                }
                encode_callable_table_entry(
                    table.sym.as_ref(),
                    property.sym.as_ref(),
                    helper,
                    arguments,
                    parameters,
                    locals,
                    context,
                    output,
                )
            }
            MemberProp::Computed(property) => {
                let mut key = Vec::new();
                encode_expression(
                    property.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut key,
                )?;
                encode_callable_table_key_call(
                    table.sym.as_ref(),
                    key,
                    arguments,
                    parameters,
                    locals,
                    context,
                    output,
                )
            }
            MemberProp::PrivateName(_) => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_table_key_call(
        table: &str,
        key: Vec<String>,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let entries = context.callable_tables.get(table)?.clone();
        let mut branches = entries
            .iter()
            .map(|(entry, helper)| {
                let mut encoded = Vec::new();
                encode_callable_table_entry(
                    table,
                    entry,
                    helper,
                    arguments,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                Some((entry.clone(), encoded))
            })
            .collect::<Option<Vec<_>>>()?;
        let kind = normalize_callable_branches(branches.iter_mut().map(|(_, branch)| branch))?;
        let missing = missing_callable(kind);
        append_string(key, output)?;
        encode_dynamic_callable_table_dispatch(
            table,
            &branches,
            arguments,
            parameters,
            locals,
            context,
            kind,
            missing,
            output,
        )
    }

    fn missing_callable(kind: JitKind) -> &'static str {
        match kind {
            JitKind::Number => "missingcalln",
            JitKind::Boolean => "missingcallb",
            JitKind::String => "missingcalls",
            JitKind::Dynamic => "missingcalldyn",
            JitKind::Array => "missingcalla",
            JitKind::Dictionary => "missingcalld",
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_dynamic_callable_table_dispatch(
        table: &str,
        branches: &[(String, Vec<String>)],
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kind: JitKind,
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((table_id, helpers)) = context.dynamic_callable_tables.get(table).cloned() else {
            encode_callable_table_branches(branches, missing, output)?;
            output.push("nip".into());
            return Some(());
        };
        let mut dynamic = helpers
            .iter()
            .map(|helper| {
                let mut encoded = Vec::new();
                encode_named_helper_target(
                    helper,
                    arguments,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                merge_jit_kinds(jit_expression_kind(&encoded)?.0, kind).map(|_| encoded)
            })
            .collect::<Option<Vec<_>>>()?;
        normalize_callable_branches(&mut dynamic)?;
        output.extend([
            "dup".into(),
            format!("c{:016x}", (table_id as f64).to_bits()),
            "callableget".into(),
            format!("c{:016x}", (-1.0f64).to_bits()),
            "!=".into(),
            "if".into(),
            "dup".into(),
            format!("c{:016x}", (table_id as f64).to_bits()),
            "callableget".into(),
        ]);
        encode_dynamic_callable_branches(&dynamic, 0, missing, output)?;
        output.push("nip".into());
        output.push("else".into());
        encode_callable_table_branches(branches, missing, output)?;
        output.push("end".into());
        output.push("nip".into());
        Some(())
    }

    fn encode_dynamic_callable_branches(
        branches: &[Vec<String>],
        selection: usize,
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((branch, remaining)) = branches.split_first() else {
            output.push(missing.into());
            return Some(());
        };
        output.extend([
            "dup".into(),
            format!("c{:016x}", (selection as f64).to_bits()),
            "==".into(),
            "if".into(),
        ]);
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_dynamic_callable_branches(remaining, selection + 1, missing, output)?;
        output.push("end".into());
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_table_entry(
        table: &str,
        key: &str,
        initial: &str,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((slot, helpers)) = context
            .callable_table_states
            .get(table)
            .and_then(|entries| entries.get(key))
            .cloned()
        else {
            return encode_named_helper_target(
                initial, arguments, parameters, locals, context, output,
            );
        };
        let mut branches = Vec::with_capacity(helpers.len());
        for helper in helpers {
            let mut encoded = Vec::new();
            encode_named_helper_target(
                &helper,
                arguments,
                parameters,
                locals,
                context,
                &mut encoded,
            )?;
            branches.push(encoded);
        }
        normalize_callable_branches(&mut branches)?;
        encode_callable_selection_branches(&branches, slot, 0, output)
    }

    fn encode_callable_selection_branches(
        branches: &[Vec<String>],
        slot: u8,
        selection: usize,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (branch, remaining) = branches.split_first()?;
        if remaining.is_empty() {
            output.extend(branch.iter().cloned());
            return Some(());
        }
        output.push(format!("c{:016x}", (slot as f64).to_bits()));
        output.push("globalget".into());
        output.push(format!("c{:016x}", (selection as f64).to_bits()));
        output.push("==".into());
        output.push("if".into());
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_callable_selection_branches(remaining, slot, selection + 1, output)?;
        output.push("end".into());
        Some(())
    }

    fn encode_callable_table_update(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (table, keys, assigned_helpers) = callable_table_assignment(expression)?;
        let Expr::Assign(assignment) = expression else {
            unreachable!()
        };
        if let Some((table_id, helpers)) = context.dynamic_callable_tables.get(&table).cloned() {
            if !assigned_helpers
                .iter()
                .all(|helper| helpers.contains(helper))
            {
                return None;
            }
            let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
                unreachable!()
            };
            match &target.prop {
                MemberProp::Ident(property) => encode_string(property.sym.as_ref(), output)?,
                MemberProp::Computed(property) => {
                    let mut key = Vec::new();
                    encode_expression(
                        property.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut key,
                    )?;
                    append_string(key, output)?;
                }
                MemberProp::PrivateName(_) => return None,
            }
            output.push(format!("c{:016x}", (table_id as f64).to_bits()));
            encode_callable_assignment_selection(
                assignment.right.as_ref(),
                &helpers,
                parameters,
                locals,
                context,
                output,
            )?;
            output.extend(["callableset".into(), "drop".into()]);
            return Some(());
        }
        let keys = keys?;
        let updates = keys
            .iter()
            .map(|key| {
                let (slot, helpers) = context.callable_table_states.get(&table)?.get(key)?;
                assigned_helpers
                    .iter()
                    .all(|helper| helpers.contains(helper))
                    .then(|| (key.clone(), *slot, helpers.clone()))
            })
            .collect::<Option<Vec<_>>>()?;
        if let [(_, slot, helpers)] = updates.as_slice() {
            output.push(format!("c{:016x}", (*slot as f64).to_bits()));
            encode_callable_assignment_selection(
                assignment.right.as_ref(),
                helpers,
                parameters,
                locals,
                context,
                output,
            )?;
            output.extend(["globalset".into(), "drop".into()]);
            return Some(());
        }
        let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
            unreachable!()
        };
        let MemberProp::Computed(property) = &target.prop else {
            return None;
        };
        let mut key = Vec::new();
        encode_expression(
            property.expr.as_ref(),
            parameters,
            locals,
            context,
            &mut key,
        )?;
        append_string(key, output)?;
        encode_callable_table_update_branches(
            &updates,
            assignment.right.as_ref(),
            parameters,
            locals,
            context,
            output,
        )?;
        output.extend(["nip".into(), "drop".into()]);
        Some(())
    }

    fn encode_callable_assignment_selection(
        expression: &Expr,
        helpers: &[String],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match expression {
            Expr::Ident(helper) => {
                let selection = helpers
                    .iter()
                    .position(|candidate| candidate == helper.sym.as_ref())?;
                output.push(format!("c{:016x}", (selection as f64).to_bits()));
            }
            Expr::Paren(parenthesized) => encode_callable_assignment_selection(
                parenthesized.expr.as_ref(),
                helpers,
                parameters,
                locals,
                context,
                output,
            )?,
            Expr::Cond(conditional) => {
                encode_condition(
                    conditional.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.push("if".into());
                encode_callable_assignment_selection(
                    conditional.cons.as_ref(),
                    helpers,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.push("else".into());
                encode_callable_assignment_selection(
                    conditional.alt.as_ref(),
                    helpers,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.push("end".into());
            }
            _ => return None,
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_table_update_branches(
        updates: &[(String, u8, Vec<String>)],
        assigned: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let ((key, slot, helpers), remaining) = updates.split_first()?;
        if remaining.is_empty() {
            output.push(format!("c{:016x}", (*slot as f64).to_bits()));
            encode_callable_assignment_selection(
                assigned, helpers, parameters, locals, context, output,
            )?;
            output.push("globalset".into());
            return Some(());
        }
        output.push("dup".into());
        encode_string(key, output)?;
        output.extend([
            "strcmp".into(),
            "c0000000000000000".into(),
            "==".into(),
            "if".into(),
            format!("c{:016x}", (*slot as f64).to_bits()),
        ]);
        encode_callable_assignment_selection(
            assigned, helpers, parameters, locals, context, output,
        )?;
        output.extend(["globalset".into(), "else".into()]);
        encode_callable_table_update_branches(
            remaining,
            assigned,
            parameters,
            locals,
            context,
            output,
        )?;
        output.push("end".into());
        Some(())
    }

    fn encode_callable_table_branches(
        branches: &[(String, Vec<String>)],
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some(((key, branch), remaining)) = branches.split_first() else {
            output.push(missing.into());
            return Some(());
        };
        output.push("dup".into());
        encode_string(key, output)?;
        output.push("strcmp".into());
        output.push("c0000000000000000".into());
        output.push("==".into());
        output.push("if".into());
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_callable_table_branches(remaining, missing, output)?;
        output.push("end".into());
        Some(())
    }

    fn encode_helper_call(
        call: &CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        encode_helper_target(
            callee.as_ref(),
            &call.args,
            parameters,
            locals,
            context,
            output,
        )
    }

    fn encode_recursive_argument(
        expression: &Expr,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<usize> {
        match ty {
            thaw_hir::HirType::Object(fields) => {
                if let Some(object) = object_literal(expression) {
                    if object.props.len() != fields.len() {
                        return None;
                    }
                    let mut slots = 0;
                    for (property, (field, field_type)) in object.props.iter().zip(fields) {
                        let (name, value) = object_property(property)?;
                        let ObjectReturnValue::Expression(value) = value else {
                            return None;
                        };
                        if &name != field {
                            return None;
                        }
                        slots += encode_recursive_argument(
                            value,
                            field_type,
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                    }
                    return Some(slots);
                }
                encode_recursive_path(&member_path(expression)?, ty, parameters, output)
            }
            thaw_hir::HirType::Tuple(types) => {
                if let Expr::Array(array) = expression {
                    if array.elems.len() != types.len() {
                        return None;
                    }
                    let mut slots = 0;
                    for (element, ty) in array.elems.iter().zip(types) {
                        let element = element.as_ref()?;
                        if element.spread.is_some() {
                            return None;
                        }
                        slots += encode_recursive_argument(
                            element.expr.as_ref(),
                            ty,
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                    }
                    return Some(slots);
                }
                encode_recursive_path(&member_path(expression)?, ty, parameters, output)
            }
            _ => {
                let expected = jit_return_kind(ty)?;
                let mut encoded = Vec::new();
                encode_expression(
                    expression,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                if jit_expression_kind(&encoded)?.0 != expected {
                    return None;
                }
                output.extend(encoded);
                Some(1)
            }
        }
    }

    fn encode_recursive_path(
        path: &str,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        output: &mut Vec<String>,
    ) -> Option<usize> {
        match ty {
            thaw_hir::HirType::Object(fields) => fields.iter().try_fold(0, |slots, (field, ty)| {
                encode_recursive_path(&format!("{path}.{field}"), ty, parameters, output)
                    .map(|count| slots + count)
            }),
            thaw_hir::HirType::Tuple(types) => {
                types.iter().enumerate().try_fold(0, |slots, (index, ty)| {
                    encode_recursive_path(&format!("{path}.{index}"), ty, parameters, output)
                        .map(|count| slots + count)
                })
            }
            _ => {
                let token = parameters.get(path)?.clone();
                (jit_expression_kind(std::slice::from_ref(&token))?.0 == jit_return_kind(ty)?)
                    .then(|| {
                        output.push(token);
                        1
                    })
            }
        }
    }

    fn resolve_callable<'a>(
        expression: &'a Expr,
        declarations: &std::collections::HashMap<String, NumericCallable<'a>>,
    ) -> Option<NumericCallable<'a>> {
        match expression {
            Expr::Fn(function) => Some(NumericCallable::Function(function.function.as_ref())),
            Expr::Arrow(function) => Some(NumericCallable::Arrow(function)),
            Expr::Ident(identifier) => declarations.get(identifier.sym.as_ref()).copied(),
            _ => None,
        }
    }

    fn resolve_callable_table(
        object: &thaw_parser::ast::ObjectLit,
        declarations: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<Vec<(String, String)>> {
        let mut names = std::collections::HashSet::new();
        let entries = object
            .props
            .iter()
            .map(|property| {
                let PropOrSpread::Prop(property) = property else {
                    return None;
                };
                let (key, value) = match property.as_ref() {
                    Prop::Shorthand(identifier) => {
                        (identifier.sym.to_string(), identifier.sym.to_string())
                    }
                    Prop::KeyValue(property) => {
                        let key = match &property.key {
                            PropName::Ident(identifier) => identifier.sym.to_string(),
                            PropName::Str(string) => {
                                string.value.to_string_lossy().into_owned()
                            }
                            _ => return None,
                        };
                        let Expr::Ident(value) = property.value.as_ref() else {
                            return None;
                        };
                        (key, value.sym.to_string())
                    }
                    _ => return None,
                };
                if !names.insert(key.clone()) || !declarations.contains_key(&value) {
                    return None;
                }
                Some((key, value))
            })
            .collect::<Option<Vec<_>>>()?;
        (!entries.is_empty()).then_some(entries)
    }

    fn finite_string_keys(expression: &Expr) -> Option<Vec<String>> {
        match expression {
            Expr::Lit(Lit::Str(key)) => Some(vec![key.value.to_string_lossy().into_owned()]),
            Expr::Paren(parenthesized) => finite_string_keys(parenthesized.expr.as_ref()),
            Expr::Cond(conditional) => {
                let mut keys = finite_string_keys(conditional.cons.as_ref())?;
                keys.extend(finite_string_keys(conditional.alt.as_ref())?);
                keys.sort();
                keys.dedup();
                Some(keys)
            }
            _ => None,
        }
    }

    fn encoded_string_literal(tokens: &[String]) -> Option<String> {
        let [token] = tokens else {
            return None;
        };
        let encoded = token.strip_prefix('t')?;
        if encoded.len() % 2 != 0 {
            return None;
        }
        let bytes = (0..encoded.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).ok())
            .collect::<Option<Vec<_>>>()?;
        String::from_utf8(bytes).ok()
    }

    fn finite_callable_names(expression: &Expr) -> Option<Vec<String>> {
        match expression {
            Expr::Ident(helper) => Some(vec![helper.sym.to_string()]),
            Expr::Paren(parenthesized) => finite_callable_names(parenthesized.expr.as_ref()),
            Expr::Cond(conditional) => {
                let mut helpers = finite_callable_names(conditional.cons.as_ref())?;
                helpers.extend(finite_callable_names(conditional.alt.as_ref())?);
                helpers.sort();
                helpers.dedup();
                Some(helpers)
            }
            _ => None,
        }
    }

    fn callable_member_helpers(
        expression: &Expr,
        context: &InlineContext<'_>,
    ) -> Option<Vec<String>> {
        let expression = match expression {
            Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
            expression => expression,
        };
        let Expr::Member(member) = expression else {
            return None;
        };
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        if let Some((_, helpers)) = context
            .dynamic_callable_tables
            .get(table.sym.as_ref())
        {
            return Some(helpers.clone());
        }
        let key = match &member.prop {
            MemberProp::Ident(property) => property.sym.as_ref(),
            MemberProp::Computed(property) => {
                let Expr::Lit(Lit::Str(key)) = property.expr.as_ref() else {
                    return None;
                };
                key.value.as_str()?
            }
            MemberProp::PrivateName(_) => return None,
        };
        context
            .callable_table_states
            .get(table.sym.as_ref())?
            .get(key)
            .map(|(_, helpers)| helpers.clone())
    }

    type CallableTableAssignment = (String, Option<Vec<String>>, Vec<String>);

    fn callable_table_assignment(
        expression: &Expr,
    ) -> Option<CallableTableAssignment> {
        let Expr::Assign(assignment) = expression else {
            return None;
        };
        if assignment.op != AssignOp::Assign {
            return None;
        }
        let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
            return None;
        };
        let Expr::Ident(table) = target.obj.as_ref() else {
            return None;
        };
        let keys = match &target.prop {
            MemberProp::Ident(property) => Some(vec![property.sym.to_string()]),
            MemberProp::Computed(property) => finite_string_keys(property.expr.as_ref()),
            MemberProp::PrivateName(_) => return None,
        };
        let helpers = finite_callable_names(assignment.right.as_ref())?;
        Some((table.sym.to_string(), keys, helpers))
    }

    fn collect_callable_table_assignments(
        statement: &Stmt,
        output: &mut Vec<CallableTableAssignment>,
    ) {
        match statement {
            Stmt::Expr(statement) => {
                if let Some(assignment) = callable_table_assignment(statement.expr.as_ref()) {
                    output.push(assignment);
                }
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_callable_table_assignments(statement, output);
                }
            }
            Stmt::If(statement) => {
                collect_callable_table_assignments(statement.cons.as_ref(), output);
                if let Some(alternate) = statement.alt.as_deref() {
                    collect_callable_table_assignments(alternate, output);
                }
            }
            Stmt::While(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::DoWhile(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::For(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::ForIn(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::ForOf(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::Switch(statement) => {
                for case in &statement.cases {
                    for statement in &case.cons {
                        collect_callable_table_assignments(statement, output);
                    }
                }
            }
            Stmt::Try(statement) => {
                for statement in &statement.block.stmts {
                    collect_callable_table_assignments(statement, output);
                }
                if let Some(handler) = &statement.handler {
                    for statement in &handler.body.stmts {
                        collect_callable_table_assignments(statement, output);
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_callable_table_assignments(statement, output);
                    }
                }
            }
            Stmt::Labeled(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            _ => {}
        }
    }

    fn exported_callable<'a>(
        assignment: &'a AssignExpr,
        export_name: &str,
        allow_default: bool,
        declarations: &std::collections::HashMap<String, NumericCallable<'a>>,
    ) -> Option<(ExportStyle, Option<NumericCallable<'a>>)> {
        if assignment.op != AssignOp::Assign {
            return None;
        }
        let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
            return None;
        };
        let is_module_exports = matches!(target.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
            && matches!(&target.prop, MemberProp::Ident(property) if property.sym == "exports");
        if is_module_exports {
            if let Expr::Object(object) = assignment.right.as_ref() {
                let mut selected = None;
                for property in &object.props {
                    let PropOrSpread::Prop(property) = property else {
                        return None;
                    };
                    let (is_target, callable) = match property.as_ref() {
                        Prop::KeyValue(property) => {
                            let is_target = match &property.key {
                                PropName::Ident(identifier) => identifier.sym == export_name,
                                PropName::Str(string) => {
                                    string.value.to_string_lossy() == export_name
                                }
                                _ => return None,
                            };
                            (
                                is_target,
                                resolve_callable(property.value.as_ref(), declarations)?,
                            )
                        }
                        Prop::Shorthand(identifier) => (
                            identifier.sym == export_name,
                            declarations
                                .get(identifier.sym.as_ref())
                                .copied()?,
                        ),
                        _ => return None,
                    };
                    if is_target && selected.replace(callable).is_some() {
                        return None;
                    }
                }
                return Some((ExportStyle::Whole, selected));
            }
            let callable = resolve_callable(assignment.right.as_ref(), declarations)?;
            return Some((
                ExportStyle::Whole,
                allow_default.then_some(callable),
            ));
        }
        let name = if let Expr::Member(object) = target.obj.as_ref() {
            if !matches!(object.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
                || !matches!(&object.prop, MemberProp::Ident(property) if property.sym == "exports")
            {
                return None;
            }
            match &target.prop {
                MemberProp::Ident(property) => property.sym.as_ref(),
                _ => return None,
            }
        } else if matches!(target.obj.as_ref(), Expr::Ident(exports) if exports.sym == "exports") {
            match &target.prop {
                MemberProp::Ident(property) => property.sym.as_ref(),
                _ => return None,
            }
        } else {
            return None;
        };
        let callable = resolve_callable(assignment.right.as_ref(), declarations)?;
        Some((
            ExportStyle::Named,
            (name == export_name).then_some(callable),
        ))
    }
    };
}
