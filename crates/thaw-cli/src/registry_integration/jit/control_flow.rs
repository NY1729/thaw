macro_rules! jit_control_flow {
    () => {
    enum LocalStep<'a> {
        Declare {
            name: &'a Ident,
            initializer: &'a Expr,
            mutable: bool,
        },
        DestructureArray {
            bindings: Vec<(usize, &'a Ident, Option<&'a Expr>)>,
            rest: Option<(usize, &'a Ident)>,
            initializer: &'a Expr,
            mutable: bool,
            assign_existing: bool,
        },
        DestructureObject {
            bindings: Vec<(String, &'a Ident, Option<&'a Expr>)>,
            initializer: &'a Expr,
            mutable: bool,
            assign_existing: bool,
        },
        Assign {
            name: &'a Ident,
            operation: AssignOp,
            value: &'a Expr,
        },
        Update {
            name: &'a Ident,
            operation: UpdateOp,
        },
        Effect(&'a Expr),
    }


    #[derive(Clone, Copy)]
    enum BreakTarget {
        Loop,
        Switch,
    }

    #[derive(Clone, Copy)]
    struct LoopControl {
        break_target: BreakTarget,
        break_finalizer_depth: usize,
        continue_finalizer_depth: usize,
        throw_finalizer_depth: usize,
        catch_active: bool,
        catch_tagged: bool,
    }

    type LoopScope<'a> = (
        &'a std::collections::HashMap<String, String>,
        &'a std::collections::HashMap<String, Vec<String>>,
        &'a std::collections::HashSet<String>,
        &'a std::collections::HashMap<String, JitKind>,
    );

    fn nested_loop_control(finalizer_depth: usize, parent: LoopControl) -> LoopControl {
        LoopControl {
            break_target: BreakTarget::Loop,
            break_finalizer_depth: finalizer_depth,
            continue_finalizer_depth: finalizer_depth,
            ..parent
        }
    }

    fn root_loop_control() -> LoopControl {
        LoopControl {
            break_target: BreakTarget::Loop,
            break_finalizer_depth: 0,
            continue_finalizer_depth: 0,
            throw_finalizer_depth: 0,
            catch_active: false,
            catch_tagged: false,
        }
    }

    fn jit_tokens_may_error(tokens: &[String]) -> bool {
        tokens.iter().any(|token| {
            matches!(
                token.as_str(),
                "repeat"
                    | "fromcodepoint"
                    | "normalize"
                    | "tofixed"
                    | "toprecision"
                    | "toexponential"
                    | "tostringradix"
                    | "rnwith"
                    | "rswith"
                    | "rbwith"
                    | "dynarraywith"
                    | "charat"
                    | "at"
                    | "concat"
                    | "numstr"
                    | "boolstr"
                    | "padend"
                    | "padstart"
                    | "split"
                    | "strarray"
                    | "fromcharcode"
                    | "replace"
                    | "replaceall"
                    | "slice"
                    | "slice2"
                    | "substring"
                    | "substring2"
                    | "tolowercase"
                    | "towellformed"
                    | "touppercase"
                    | "trim"
                    | "trimend"
                    | "trimstart"
                    | "rnjoin"
                    | "rbjoin"
                    | "rsjoin"
                    | "arrayslice"
                    | "arrayconcat"
                    | "arrayreversed"
                    | "arrayreverse"
                    | "rnsorted"
                    | "rssorted"
                    | "rbsorted"
                    | "rnsort"
                    | "rnsortedasc"
                    | "rnsorteddesc"
                    | "rnsortasc"
                    | "rnsortdesc"
                    | "rssorteddesc"
                    | "rssortdesc"
                    | "rssort"
                    | "rbsort"
                    | "rnfill"
                    | "rsfill"
                    | "rbfill"
                    | "arraycopywithin"
                    | "arraysplice"
                    | "arraytospliced"
                    | "dynarraysplice"
                    | "dynarraytospliced"
                    | "dynarrayappend"
                    | "dynarraysometruthy"
                    | "dynarrayeverytruthy"
                    | "dynarrayfindtruthy"
                    | "dynarrayfindindextruthy"
                    | "dynarrayfindlasttruthy"
                    | "dynarrayfindlastindextruthy"
                    | "dynarrayfiltertruthy"
                    | "dynarraymaptonumber"
                    | "dynarraymaptoboolean"
                    | "dynarraymaptostring"
                    | "dynarraymapidentity"
                    | "rnpush"
                    | "rspush"
                    | "rbpush"
                    | "dynarraypush"
                    | "rnunshift"
                    | "rsunshift"
                    | "rbunshift"
                    | "dynarrayunshift"
                    | "rnset"
                    | "rnpostset"
                    | "rsset"
                    | "rbset"
                    | "dynarrayset"
                    | "rnpop"
                    | "rspop"
                    | "rbpop"
                    | "rnshift"
                    | "rsshift"
                    | "rbshift"
                    | "dynarraypop"
                    | "dynarrayshift"
                    | "dkeys"
                    | "dnvalues"
                    | "dbvalues"
                    | "dsvalues"
                    | "dnentries"
                    | "dbentries"
                    | "dsentries"
                    | "dnfromentries"
                    | "dbfromentries"
                    | "dsfromentries"
                    | "missingcalln"
                    | "missingcallb"
                    | "missingcalls"
                    | "missingcalla"
                    | "missingcalld"
                    | "globalget"
                    | "globalinit"
                    | "globalset"
                    | "callableget"
                    | "callableset"
            )
                || token.starts_with("rnreduce")
                || token.starts_with("rnreduceright")
                || token.starts_with("rnfilter")
                || token.starts_with("dynarray")
                || token.contains("mapjit")
                || token.contains("filterjit")
        })
    }

    fn insert_jit_error_checks(output: &mut Vec<String>, start: usize) {
        if !jit_tokens_may_error(&output[start..]) {
            return;
        }
        let tokens = output.split_off(start);
        for token in tokens {
            let may_error = jit_tokens_may_error(std::slice::from_ref(&token));
            output.push(token);
            if may_error {
                output.push("checkerror".into());
            }
        }
    }

    fn destructuring_property_name(property: &PropName) -> Option<String> {
        match property {
            PropName::Ident(name) => Some(name.sym.to_string()),
            PropName::Str(name) => Some(name.value.to_string_lossy().into_owned()),
            PropName::Computed(name) => match name.expr.as_ref() {
                Expr::Lit(Lit::Str(name)) => Some(name.value.to_string_lossy().into_owned()),
                _ => None,
            },
            _ => None,
        }
    }

    fn collect_fixed_object_bindings<'a>(
        pattern: &'a Pat,
        path: &str,
        bindings: &mut Vec<(String, &'a Ident, Option<&'a Expr>)>,
    ) -> Option<()> {
        match pattern {
            Pat::Ident(name) => bindings.push((path.into(), &name.id, None)),
            Pat::Assign(assignment) => {
                let Pat::Ident(name) = assignment.left.as_ref() else {
                    return None;
                };
                bindings.push((
                    path.into(),
                    &name.id,
                    Some(assignment.right.as_ref()),
                ));
            }
            Pat::Object(pattern) => {
                collect_fixed_object_pattern(pattern, path, bindings)?;
            }
            Pat::Array(pattern) => collect_fixed_array_pattern(pattern, path, bindings)?,
            _ => return None,
        }
        Some(())
    }

    fn collect_fixed_array_pattern<'a>(
        pattern: &'a thaw_parser::ast::ArrayPat,
        path: &str,
        bindings: &mut Vec<(String, &'a Ident, Option<&'a Expr>)>,
    ) -> Option<()> {
        for (index, element) in pattern.elems.iter().enumerate() {
            let Some(element) = element else {
                continue;
            };
            if matches!(element, Pat::Rest(_)) {
                return None;
            }
            collect_fixed_object_bindings(element, &format!("{path}.{index}"), bindings)?;
        }
        Some(())
    }

    fn materialize_helper_value(
        mut encoded: Vec<String>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        output: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        let (kind, _) = jit_expression_kind(&encoded).or_else(|| {
            let mut stacked = output.clone();
            stacked.extend(encoded.iter().cloned());
            stacked.extend(std::iter::repeat_n("nip".into(), kinds.len()));
            jit_expression_kind(&stacked)
        })?;
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
        output.append(&mut encoded);
        if kind == JitKind::Boolean {
            output.push("asbool".into());
        } else if kind == JitKind::Array {
            output.push("arrayhandle".into());
        }
        kinds.insert(format!("\0helper-{index}"), kind);
        Some(vec![format!("{prefix}{index}")])
    }

    fn helper_control_kinds(
        kinds: &std::collections::HashMap<String, JitKind>,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<std::collections::HashMap<String, JitKind>> {
        let mut control = kinds.clone();
        for (name, value) in locals {
            let Some(token) = value.first() else {
                continue;
            };
            let Some(kind) = runtime_local_kind(token) else {
                continue;
            };
            let index = loop_local_index(token)?;
            if control.remove(&format!("\0literal-{index}")).is_none() {
                control.remove(&format!("\0helper-{index}"));
            }
            control.insert(name.clone(), kind);
        }
        (control.len() == kinds.len()).then_some(control)
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_fixed_literal(
        expression: &Expr,
        path: &str,
        requested: &std::collections::HashSet<String>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        materialized: &mut std::collections::HashMap<String, Vec<String>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match expression {
            Expr::Paren(parenthesized) => materialize_fixed_literal(
                parenthesized.expr.as_ref(),
                path,
                requested,
                parameters,
                locals,
                context,
                kinds,
                materialized,
                output,
            ),
            Expr::Object(object) if !requested.contains(path) => {
                for property in &object.props {
                    let PropOrSpread::Prop(property) = property else {
                        return None;
                    };
                    let (name, value) = match property.as_ref() {
                        Prop::Shorthand(name) => (
                            name.sym.to_string(),
                            locals
                                .get(name.sym.as_ref())
                                .map(|_| Expr::Ident(name.clone()))
                                .or_else(|| {
                                    parameters
                                        .get(name.sym.as_ref())
                                        .map(|_| Expr::Ident(name.clone()))
                                })?,
                        ),
                        Prop::KeyValue(property) => (
                            destructuring_property_name(&property.key)?,
                            property.value.as_ref().clone(),
                        ),
                        _ => return None,
                    };
                    materialize_fixed_literal(
                        &value,
                        &format!("{path}.{name}"),
                        requested,
                        parameters,
                        locals,
                        context,
                        kinds,
                        materialized,
                        output,
                    )?;
                }
                Some(())
            }
            Expr::Array(array) if !requested.contains(path) => {
                for (index, element) in array.elems.iter().enumerate() {
                    let Some(element) = element else {
                        continue;
                    };
                    if element.spread.is_some() {
                        return None;
                    }
                    materialize_fixed_literal(
                        element.expr.as_ref(),
                        &format!("{path}.{index}"),
                        requested,
                        parameters,
                        locals,
                        context,
                        kinds,
                        materialized,
                        output,
                    )?;
                }
                Some(())
            }
            expression => {
                let mut encoded = Vec::new();
                encode_expression(expression, parameters, locals, context, &mut encoded)?;
                let (kind, _) = jit_expression_kind(&encoded)
                    .or_else(|| {
                        let mut stacked = output.clone();
                        stacked.extend(encoded.iter().cloned());
                        stacked.extend(std::iter::repeat_n("nip".into(), kinds.len()));
                        jit_expression_kind(&stacked)
                    })?;
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
                kinds.insert(format!(" literal-{index}"), kind);
                materialized.insert(path.into(), vec![format!("{prefix}{index}")]);
                Some(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_helper_switch_cases(
        cases: &[thaw_parser::ast::SwitchCase],
        discriminant: &[String],
        requested: &std::collections::HashSet<String>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        context: &mut InlineContext<'_>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        materialized: &mut std::collections::HashMap<String, Vec<String>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (case, remaining) = cases.split_first()?;
        let statements = case.cons.iter().collect::<Vec<_>>();
        let Some(test) = case.test.as_deref() else {
            if !remaining.is_empty() {
                return None;
            }
            return materialize_helper_returns(
                &statements,
                requested,
                parameters,
                locals,
                mutable,
                context,
                kinds,
                materialized,
                output,
            );
        };
        let discriminant_kind = jit_expression_kind(discriminant)?.0;
        let mut condition = discriminant.to_vec();
        let mut case_value = Vec::new();
        encode_expression(test, parameters, locals, context, &mut case_value)?;
        if jit_expression_kind(&case_value)?.0 != discriminant_kind {
            return None;
        }
        condition.extend(case_value);
        condition.push(
            match discriminant_kind {
                JitKind::Number | JitKind::Boolean => "==",
                JitKind::String => "strsame",
                JitKind::Dynamic => "dynseq",
                JitKind::Array => "refsame",
                JitKind::Dictionary => return None,
            }
            .into(),
        );
        let mut consequent_kinds = kinds.clone();
        let mut consequent_values = std::collections::HashMap::new();
        let mut consequent_output = Vec::new();
        materialize_helper_returns(
            &statements,
            requested,
            parameters,
            locals,
            mutable,
            context,
            &mut consequent_kinds,
            &mut consequent_values,
            &mut consequent_output,
        )?;
        let mut alternate_kinds = kinds.clone();
        let mut alternate_values = std::collections::HashMap::new();
        let mut alternate_output = Vec::new();
        materialize_helper_switch_cases(
            remaining,
            discriminant,
            requested,
            parameters,
            locals,
            mutable,
            context,
            &mut alternate_kinds,
            &mut alternate_values,
            &mut alternate_output,
        )?;
        if consequent_kinds != alternate_kinds || consequent_values != alternate_values {
            return None;
        }
        output.extend(condition);
        output.push("if".into());
        output.extend(consequent_output);
        output.push("else".into());
        output.extend(alternate_output);
        output.push("end".into());
        *kinds = consequent_kinds;
        *materialized = consequent_values;
        Some(())
    }

    fn contains_aggregate_return(statement: &Stmt) -> bool {
        match statement {
            Stmt::Return(returned) => returned.arg.is_some(),
            Stmt::Block(block) => block.stmts.iter().any(contains_aggregate_return),
            Stmt::If(branch) => {
                contains_aggregate_return(branch.cons.as_ref())
                    || branch
                        .alt
                        .as_deref()
                        .is_some_and(contains_aggregate_return)
            }
            Stmt::While(statement) => contains_aggregate_return(statement.body.as_ref()),
            Stmt::DoWhile(statement) => contains_aggregate_return(statement.body.as_ref()),
            Stmt::For(statement) => contains_aggregate_return(statement.body.as_ref()),
            Stmt::ForOf(statement) => contains_aggregate_return(statement.body.as_ref()),
            Stmt::ForIn(statement) => contains_aggregate_return(statement.body.as_ref()),
            Stmt::Switch(statement) => statement
                .cases
                .iter()
                .flat_map(|case| &case.cons)
                .any(contains_aggregate_return),
            Stmt::Try(statement) => {
                statement.block.stmts.iter().any(contains_aggregate_return)
                    || statement.handler.as_ref().is_some_and(|handler| {
                        handler.body.stmts.iter().any(contains_aggregate_return)
                    })
                    || statement.finalizer.as_ref().is_some_and(|finalizer| {
                        finalizer.stmts.iter().any(contains_aggregate_return)
                    })
            }
            Stmt::Labeled(statement) => contains_aggregate_return(statement.body.as_ref()),
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_aggregate_return_effects(
        statement: &Stmt,
        requested: &std::collections::HashSet<String>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        control_kinds: &std::collections::HashMap<String, JitKind>,
        result_base_kinds: &std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        expected_kinds: &mut Option<std::collections::HashMap<String, JitKind>>,
        expected_values: &mut Option<std::collections::HashMap<String, Vec<String>>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match statement {
            Stmt::Return(returned) => {
                let mut returned_kinds = result_base_kinds.clone();
                let mut returned_values = std::collections::HashMap::new();
                materialize_fixed_literal(
                    returned.arg.as_deref()?,
                    "",
                    requested,
                    parameters,
                    locals,
                    context,
                    &mut returned_kinds,
                    &mut returned_values,
                    output,
                )?;
                if expected_kinds
                    .as_ref()
                    .is_some_and(|expected| expected != &returned_kinds)
                    || expected_values
                        .as_ref()
                        .is_some_and(|expected| expected != &returned_values)
                {
                    return None;
                }
                let result_count = returned_kinds.len().checked_sub(result_base_kinds.len())?;
                *expected_kinds = Some(returned_kinds);
                *expected_values = Some(returned_values);
                output.push(format!("resultreturn{result_count}"));
                Some(())
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    encode_aggregate_return_effects(
                        statement,
                        requested,
                        parameters,
                        locals,
                        mutable,
                        control_kinds,
                        result_base_kinds,
                        context,
                        expected_kinds,
                        expected_values,
                        output,
                    )?;
                    if matches!(statement, Stmt::Return(_)) {
                        break;
                    }
                }
                Some(())
            }
            Stmt::If(branch) => {
                encode_condition(branch.test.as_ref(), parameters, locals, context, output)?;
                let narrowed = narrowed_locals(
                    branch.test.as_ref(),
                    parameters,
                    locals,
                    context.helpers,
                );
                let consequent_locals = narrowed.as_ref().map_or(locals, |locals| &locals.0);
                let alternate_locals = narrowed.as_ref().map_or(locals, |locals| &locals.1);
                output.push("guard".into());
                encode_aggregate_return_effects(
                    branch.cons.as_ref(),
                    requested,
                    parameters,
                    consequent_locals,
                    mutable,
                    control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
                    output,
                )?;
                if let Some(alternate) = branch.alt.as_deref() {
                    output.push("guardelse".into());
                    encode_aggregate_return_effects(
                        alternate,
                        requested,
                        parameters,
                        alternate_locals,
                        mutable,
                        control_kinds,
                        result_base_kinds,
                        context,
                        expected_kinds,
                        expected_values,
                        output,
                    )?;
                }
                output.push("guardend".into());
                Some(())
            }
            Stmt::While(loop_statement)
                if contains_aggregate_return(loop_statement.body.as_ref()) =>
            {
                output.extend(["loop".into(), "looptail".into()]);
                encode_condition(
                    loop_statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.push("while".into());
                context.loop_depth += 1;
                let body_result = encode_aggregate_return_effects(
                    loop_statement.body.as_ref(),
                    requested,
                    parameters,
                    locals,
                    mutable,
                    control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
                    output,
                );
                context.loop_depth -= 1;
                body_result?;
                output.push("loopend".into());
                Some(())
            }
            Stmt::DoWhile(loop_statement)
                if contains_aggregate_return(loop_statement.body.as_ref()) =>
            {
                output.push("loop".into());
                context.loop_depth += 1;
                let body_result = encode_aggregate_return_effects(
                    loop_statement.body.as_ref(),
                    requested,
                    parameters,
                    locals,
                    mutable,
                    control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
                    output,
                );
                context.loop_depth -= 1;
                body_result?;
                output.push("looptail".into());
                encode_condition(
                    loop_statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.extend(["while".into(), "loopend".into()]);
                Some(())
            }
            Stmt::For(loop_statement)
                if contains_aggregate_return(loop_statement.body.as_ref()) =>
            {
                let mut nested_locals = locals.clone();
                let mut nested_mutable = mutable.clone();
                let mut nested_kinds = control_kinds.clone();
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
                    None => {}
                }
                output.push("loop".into());
                if let Some(test) = loop_statement.test.as_deref() {
                    encode_condition(test, parameters, &nested_locals, context, output)?;
                } else {
                    output.extend([
                        format!("c{:016x}", 1.0f64.to_bits()),
                        "asbool".into(),
                    ]);
                }
                output.push("while".into());
                let nested_control_kinds = helper_control_kinds(&nested_kinds, &nested_locals)?;
                context.loop_depth += 1;
                let body_result = encode_aggregate_return_effects(
                    loop_statement.body.as_ref(),
                    requested,
                    parameters,
                    &nested_locals,
                    &nested_mutable,
                    &nested_control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
                    output,
                );
                context.loop_depth -= 1;
                body_result?;
                output.push("looptail".into());
                if let Some(update) = loop_statement.update.as_deref() {
                    encode_loop_expression(
                        update,
                        parameters,
                        &nested_locals,
                        &nested_mutable,
                        &nested_control_kinds,
                        context,
                        output,
                    )?;
                }
                output.push("loopend".into());
                output.extend(std::iter::repeat_n(
                    "drop".into(),
                    nested_kinds.len().checked_sub(control_kinds.len())?,
                ));
                Some(())
            }
            Stmt::ForOf(loop_statement)
                if !loop_statement.is_await
                    && contains_aggregate_return(loop_statement.body.as_ref()) =>
            {
                let mut nested_locals = locals.clone();
                let mut nested_mutable = mutable.clone();
                let mut nested_kinds = control_kinds.clone();
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
                if jit_expression_kind(&source)?.0 != JitKind::Array {
                    return None;
                }
                let array = array_prefix(&source)?;
                let element_kind = match array {
                    "rn" => JitKind::Number,
                    "rb" => JitKind::Boolean,
                    "rs" => JitKind::String,
                    _ => return None,
                };
                let local_prefix = format!("{array}l");
                let source_local = if let [source] = source.as_slice() {
                    source.starts_with(&local_prefix).then(|| source.clone())
                } else {
                    None
                }
                .unwrap_or_else(|| {
                    let source_index = nested_kinds.len();
                    output.extend(source);
                    nested_kinds.insert(
                        format!("\0forof-source-{source_index}"),
                        JitKind::Array,
                    );
                    format!("{array}l{source_index}")
                });
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
                let nested_control_kinds = helper_control_kinds(&nested_kinds, &nested_locals)?;
                context.loop_depth += 1;
                let body_result = encode_aggregate_return_effects(
                    loop_statement.body.as_ref(),
                    requested,
                    parameters,
                    &nested_locals,
                    &nested_mutable,
                    &nested_control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
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
                    nested_kinds.len().checked_sub(control_kinds.len())?,
                ));
                Some(())
            }
            Stmt::ForIn(loop_statement)
                if contains_aggregate_return(loop_statement.body.as_ref()) =>
            {
                let mut nested_locals = locals.clone();
                let mut nested_mutable = mutable.clone();
                let mut nested_kinds = control_kinds.clone();
                let mut source = Vec::new();
                encode_expression(
                    loop_statement.right.as_ref(),
                    parameters,
                    &nested_locals,
                    context,
                    &mut source,
                )?;
                if jit_expression_kind(&source)?.0 != JitKind::Dictionary {
                    return None;
                }
                let dictionary = dictionary_prefix(&source)?;
                let local_prefix = format!("{dictionary}l");
                let source_local = if let [source] = source.as_slice() {
                    source.starts_with(&local_prefix).then(|| source.clone())
                } else {
                    None
                }
                .unwrap_or_else(|| {
                    let source_index = nested_kinds.len();
                    output.extend(source);
                    nested_kinds.insert(
                        format!("\0forin-source-{source_index}"),
                        JitKind::Dictionary,
                    );
                    format!("{dictionary}l{source_index}")
                });
                let index = nested_kinds.len();
                output.push(format!("c{:016x}", 0.0f64.to_bits()));
                nested_kinds.insert(format!("\0forin-index-{index}"), JitKind::Number);
                let index_local = format!("ln{index}");
                let key_index = match &loop_statement.left {
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
                        encode_string("", output)?;
                        let key_index = nested_kinds.len();
                        nested_locals
                            .insert(name.id.sym.to_string(), vec![format!("ls{key_index}")]);
                        nested_kinds.insert(name.id.sym.to_string(), JitKind::String);
                        if declaration.kind != VarDeclKind::Const {
                            nested_mutable.insert(name.id.sym.to_string());
                        }
                        key_index
                    }
                    ForHead::Pat(pattern) => {
                        let Pat::Ident(name) = pattern.as_ref() else {
                            return None;
                        };
                        if !nested_mutable.contains(name.id.sym.as_ref())
                            || nested_kinds.get(name.id.sym.as_ref())? != &JitKind::String
                        {
                            return None;
                        }
                        loop_local_index(nested_locals.get(name.id.sym.as_ref())?.first()?)?
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
                    source_local,
                    index_local.clone(),
                    "dkeyat".into(),
                    format!("setl{key_index}"),
                ]);
                let nested_control_kinds = helper_control_kinds(&nested_kinds, &nested_locals)?;
                context.loop_depth += 1;
                let body_result = encode_aggregate_return_effects(
                    loop_statement.body.as_ref(),
                    requested,
                    parameters,
                    &nested_locals,
                    &nested_mutable,
                    &nested_control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
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
                    nested_kinds.len().checked_sub(control_kinds.len())?,
                ));
                Some(())
            }
            Stmt::Switch(switch_statement) if contains_aggregate_return(statement) => {
                let mut discriminant = Vec::new();
                encode_expression(
                    switch_statement.discriminant.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut discriminant,
                )?;
                let discriminant_kind =
                    if boolean_literal(switch_statement.discriminant.as_ref()) {
                        discriminant.push("asbool".into());
                        JitKind::Boolean
                    } else {
                        jit_expression_kind(&discriminant)?.0
                    };
                if matches!(discriminant_kind, JitKind::Array | JitKind::Dictionary) {
                    return None;
                }
                output.extend(discriminant);
                output.push("switch".into());
                for case in &switch_statement.cases {
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
                        if discriminant_kind == JitKind::String && test_kind == JitKind::String {
                            output.extend([
                                "strcmp".into(),
                                format!("c{:016x}", 0.0f64.to_bits()),
                                "==".into(),
                            ]);
                        } else if discriminant_kind == test_kind
                            && matches!(
                                discriminant_kind,
                                JitKind::Number | JitKind::Boolean
                            )
                        {
                            output.push("==".into());
                        } else {
                            output.push("strictfalse".into());
                        }
                        output.push("casebody".into());
                    } else {
                        output.push("default".into());
                    }
                    for case_statement in &case.cons {
                        if matches!(case_statement, Stmt::Break(statement) if statement.label.is_none())
                        {
                            output.push("switchbreak".into());
                            break;
                        }
                        encode_aggregate_return_effects(
                            case_statement,
                            requested,
                            parameters,
                            locals,
                            mutable,
                            control_kinds,
                            result_base_kinds,
                            context,
                            expected_kinds,
                            expected_values,
                            output,
                        )?;
                        if matches!(case_statement, Stmt::Return(_)) {
                            break;
                        }
                    }
                }
                output.push("switchend".into());
                Some(())
            }
            Stmt::Try(try_statement)
                if try_statement.handler.is_some()
                    && try_statement.finalizer.as_ref().is_none_or(|finalizer| {
                        finalizer
                            .stmts
                            .iter()
                            .all(|statement| matches!(statement, Stmt::Expr(_)))
                    })
                    && try_statement
                        .block
                        .stmts
                        .iter()
                        .any(contains_aggregate_return) =>
            {
                let mut merged_kinds = control_kinds.clone();
                let mut merged_values = std::collections::HashMap::new();
                let mut merged_output = Vec::new();
                materialize_helper_returns(
                    std::slice::from_ref(&statement),
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut merged_kinds,
                    &mut merged_values,
                    &mut merged_output,
                )?;
                let mut leaves = merged_values
                    .iter()
                    .map(|(path, tokens)| {
                        let [token] = tokens.as_slice() else {
                            return None;
                        };
                        Some((
                            loop_local_index(token)?,
                            path.clone(),
                            runtime_local_kind(token)?,
                            token
                                .get(..token.find(|character: char| character.is_ascii_digit())?)?
                                .to_string(),
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                leaves.sort_by_key(|leaf| leaf.0);
                let first_leaf = merged_kinds.len().checked_sub(leaves.len())?;
                if leaves
                    .iter()
                    .enumerate()
                    .any(|(offset, leaf)| leaf.0 != first_leaf + offset)
                {
                    return None;
                }
                let mut returned_kinds = result_base_kinds.clone();
                let mut returned_values = std::collections::HashMap::new();
                for (_, path, kind, prefix) in leaves {
                    let index = returned_kinds.len();
                    returned_kinds.insert(format!("\0literal-{index}"), kind);
                    returned_values.insert(path, vec![format!("{prefix}{index}")]);
                }
                if expected_kinds
                    .as_ref()
                    .is_some_and(|expected| expected != &returned_kinds)
                    || expected_values
                        .as_ref()
                        .is_some_and(|expected| expected != &returned_values)
                {
                    return None;
                }
                let result_count = returned_kinds.len().checked_sub(result_base_kinds.len())?;
                *expected_kinds = Some(returned_kinds);
                *expected_values = Some(returned_values);
                output.extend(merged_output);
                output.push(format!("resultreturn{result_count}"));
                Some(())
            }
            Stmt::Try(try_statement)
                if try_statement.handler.is_none()
                    && try_statement.finalizer.as_ref().is_some_and(|finalizer| {
                        finalizer
                            .stmts
                            .iter()
                            .all(|statement| matches!(statement, Stmt::Expr(_)))
                    })
                    && try_statement
                        .block
                        .stmts
                        .iter()
                        .any(contains_aggregate_return) =>
            {
                let finalizer = try_statement.finalizer.as_ref()?;
                let mut finalizer_output = Vec::new();
                encode_loop_effects(
                    &Stmt::Block(finalizer.clone()),
                    parameters,
                    locals,
                    mutable,
                    (control_kinds, root_loop_control(), &[]),
                    context,
                    &mut finalizer_output,
                )?;
                let mut body_output = Vec::new();
                encode_aggregate_return_effects(
                    &Stmt::Block(try_statement.block.clone()),
                    requested,
                    parameters,
                    locals,
                    mutable,
                    control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
                    &mut body_output,
                )?;
                for token in body_output {
                    if token.starts_with("resultreturn") {
                        output.extend(finalizer_output.iter().cloned());
                    }
                    output.push(token);
                }
                output.extend(finalizer_output);
                Some(())
            }
            Stmt::Labeled(labeled) if contains_aggregate_return(labeled.body.as_ref()) => {
                let target = context.loop_depth;
                context
                    .loop_labels
                    .push((labeled.label.sym.to_string(), target));
                let result = encode_aggregate_return_effects(
                    labeled.body.as_ref(),
                    requested,
                    parameters,
                    locals,
                    mutable,
                    control_kinds,
                    result_base_kinds,
                    context,
                    expected_kinds,
                    expected_values,
                    output,
                );
                context.loop_labels.pop();
                result
            }
            Stmt::Break(break_statement) if break_statement.label.is_some() => {
                let label = break_statement.label.as_ref()?.sym.to_string();
                let target = context
                    .loop_labels
                    .iter()
                    .rev()
                    .find(|(name, _)| name == &label)
                    .map(|(_, depth)| *depth)?;
                let distance = context
                    .loop_depth
                    .checked_sub(target.checked_add(1)?)?;
                output.push(format!("break{distance}"));
                Some(())
            }
            Stmt::Continue(continue_statement) if continue_statement.label.is_some() => {
                let label = continue_statement.label.as_ref()?.sym.to_string();
                let target = context
                    .loop_labels
                    .iter()
                    .rev()
                    .find(|(name, _)| name == &label)
                    .map(|(_, depth)| *depth)?;
                let distance = context
                    .loop_depth
                    .checked_sub(target.checked_add(1)?)?;
                output.push(format!("continue{distance}"));
                Some(())
            }
            Stmt::For(_)
            | Stmt::ForIn(_)
            | Stmt::ForOf(_)
            | Stmt::Switch(_)
            | Stmt::Try(_)
            | Stmt::Labeled(_) => None,
            statement => encode_loop_effects(
                statement,
                parameters,
                locals,
                mutable,
                (
                    control_kinds,
                    nested_loop_control(0, root_loop_control()),
                    &[],
                ),
                context,
                output,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_helper_returns(
        statements: &[&Stmt],
        requested: &std::collections::HashSet<String>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        context: &mut InlineContext<'_>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        materialized: &mut std::collections::HashMap<String, Vec<String>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (statement, rest) = statements.split_first()?;
        match statement {
            Stmt::Block(block) => {
                let nested = block.stmts.iter().chain(rest.iter().copied()).collect::<Vec<_>>();
                materialize_helper_returns(
                    &nested,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    kinds,
                    materialized,
                    output,
                )
            }
            Stmt::Labeled(labeled) if contains_aggregate_return(labeled.body.as_ref()) => {
                let target = context.loop_depth;
                context
                    .loop_labels
                    .push((labeled.label.sym.to_string(), target));
                let nested = std::iter::once(labeled.body.as_ref())
                    .chain(rest.iter().copied())
                    .collect::<Vec<_>>();
                let result = materialize_helper_returns(
                    &nested,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    kinds,
                    materialized,
                    output,
                );
                context.loop_labels.pop();
                result
            }
            Stmt::Return(returned) if rest.is_empty() => materialize_fixed_literal(
                returned.arg.as_deref()?,
                "",
                requested,
                parameters,
                locals,
                context,
                kinds,
                materialized,
                output,
            ),
            Stmt::While(statement)
                if !rest.is_empty() && contains_aggregate_return(statement.body.as_ref()) =>
            {
                let mut loop_output = vec!["resultstart".into(), "loop".into()];
                encode_condition(
                    statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut loop_output,
                )?;
                loop_output.push("while".into());
                let control_kinds = helper_control_kinds(kinds, locals)?;
                let mut early_kinds = None;
                let mut early_values = None;
                context.loop_depth += 1;
                let body_result = encode_aggregate_return_effects(
                    statement.body.as_ref(),
                    requested,
                    parameters,
                    locals,
                    mutable,
                    &control_kinds,
                    kinds,
                    context,
                    &mut early_kinds,
                    &mut early_values,
                    &mut loop_output,
                );
                context.loop_depth -= 1;
                body_result?;
                let early_kinds = early_kinds?;
                let early_values = early_values?;
                loop_output.extend(["looptail".into(), "loopend".into()]);
                let mut fallback_kinds = kinds.clone();
                let mut fallback_values = std::collections::HashMap::new();
                let mut fallback_output = Vec::new();
                materialize_helper_returns(
                    rest,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut fallback_kinds,
                    &mut fallback_values,
                    &mut fallback_output,
                )?;
                if early_kinds != fallback_kinds || early_values != fallback_values {
                    return None;
                }
                output.extend(loop_output);
                output.extend(fallback_output);
                output.push("resultend".into());
                *kinds = fallback_kinds;
                *materialized = fallback_values;
                Some(())
            }
            Stmt::DoWhile(statement)
                if !rest.is_empty() && contains_aggregate_return(statement.body.as_ref()) =>
            {
                let mut loop_output = vec!["resultstart".into(), "loop".into()];
                let control_kinds = helper_control_kinds(kinds, locals)?;
                let mut early_kinds = None;
                let mut early_values = None;
                context.loop_depth += 1;
                let body_result = encode_aggregate_return_effects(
                    statement.body.as_ref(),
                    requested,
                    parameters,
                    locals,
                    mutable,
                    &control_kinds,
                    kinds,
                    context,
                    &mut early_kinds,
                    &mut early_values,
                    &mut loop_output,
                );
                context.loop_depth -= 1;
                body_result?;
                let early_kinds = early_kinds?;
                let early_values = early_values?;
                loop_output.push("looptail".into());
                encode_condition(
                    statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut loop_output,
                )?;
                loop_output.extend(["while".into(), "loopend".into()]);
                let mut fallback_kinds = kinds.clone();
                let mut fallback_values = std::collections::HashMap::new();
                let mut fallback_output = Vec::new();
                materialize_helper_returns(
                    rest,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut fallback_kinds,
                    &mut fallback_values,
                    &mut fallback_output,
                )?;
                if early_kinds != fallback_kinds || early_values != fallback_values {
                    return None;
                }
                output.extend(loop_output);
                output.extend(fallback_output);
                output.push("resultend".into());
                *kinds = fallback_kinds;
                *materialized = fallback_values;
                Some(())
            }
            Stmt::For(statement)
                if !rest.is_empty()
                    && contains_aggregate_return(statement.body.as_ref()) =>
            {
                let mut loop_locals = locals.clone();
                let mut loop_mutable = mutable.clone();
                let mut loop_kinds = kinds.clone();
                match statement.init.as_ref() {
                    Some(VarDeclOrExpr::Expr(initializer)) => encode_loop_expression(
                        initializer.as_ref(),
                        parameters,
                        &loop_locals,
                        &loop_mutable,
                        &loop_kinds,
                        context,
                        output,
                    )?,
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
                                &mut loop_locals,
                                &mut loop_mutable,
                                &mut loop_kinds,
                                context,
                                output,
                            )?;
                        }
                    }
                    None => {}
                }
                let mut loop_output = vec!["resultstart".into(), "loop".into()];
                if let Some(test) = statement.test.as_deref() {
                    encode_condition(
                        test,
                        parameters,
                        &loop_locals,
                        context,
                        &mut loop_output,
                    )?;
                } else {
                    loop_output.extend([
                        format!("c{:016x}", 1.0f64.to_bits()),
                        "asbool".into(),
                    ]);
                }
                loop_output.push("while".into());
                let control_kinds = helper_control_kinds(&loop_kinds, &loop_locals)?;
                let mut early_kinds = None;
                let mut early_values = None;
                encode_aggregate_return_effects(
                    statement.body.as_ref(),
                    requested,
                    parameters,
                    &loop_locals,
                    &loop_mutable,
                    &control_kinds,
                    &loop_kinds,
                    context,
                    &mut early_kinds,
                    &mut early_values,
                    &mut loop_output,
                )?;
                let early_kinds = early_kinds?;
                let early_values = early_values?;
                loop_output.push("looptail".into());
                if let Some(update) = statement.update.as_deref() {
                    encode_loop_expression(
                        update,
                        parameters,
                        &loop_locals,
                        &loop_mutable,
                        &control_kinds,
                        context,
                        &mut loop_output,
                    )?;
                }
                loop_output.push("loopend".into());
                let mut fallback_kinds = loop_kinds.clone();
                let mut fallback_values = std::collections::HashMap::new();
                let mut fallback_output = Vec::new();
                materialize_helper_returns(
                    rest,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut fallback_kinds,
                    &mut fallback_values,
                    &mut fallback_output,
                )?;
                if early_kinds != fallback_kinds || early_values != fallback_values {
                    return None;
                }
                output.extend(loop_output);
                output.extend(fallback_output);
                output.push("resultend".into());
                for (slot, name) in loop_kinds
                    .keys()
                    .filter(|name| !kinds.contains_key(*name))
                    .enumerate()
                {
                    let kind = fallback_kinds.remove(name)?;
                    fallback_kinds.insert(
                        format!("\0for-result-{slot}-{}", fallback_kinds.len()),
                        kind,
                    );
                }
                *kinds = fallback_kinds;
                *materialized = fallback_values;
                Some(())
            }
            Stmt::ForOf(statement)
                if !statement.is_await
                    && !rest.is_empty()
                    && contains_aggregate_return(statement.body.as_ref()) =>
            {
                let mut loop_locals = locals.clone();
                let mut loop_mutable = mutable.clone();
                let mut loop_kinds = kinds.clone();
                let mut source = Vec::new();
                encode_expression(
                    statement.right.as_ref(),
                    parameters,
                    &loop_locals,
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
                let array = array_prefix(&source)?;
                let element_kind = match array {
                    "rn" => JitKind::Number,
                    "rb" => JitKind::Boolean,
                    "rs" => JitKind::String,
                    _ => return None,
                };
                let local_prefix = format!("{array}l");
                let source_local = if let [source] = source.as_slice() {
                    source
                        .starts_with(&local_prefix)
                        .then(|| source.clone())
                } else {
                    None
                }
                .unwrap_or_else(|| {
                    let source_index = loop_kinds.len();
                    output.extend(source);
                    loop_kinds.insert(
                        format!("\0forof-source-{source_index}"),
                        JitKind::Array,
                    );
                    format!("{array}l{source_index}")
                });
                let index = loop_kinds.len();
                output.push(format!("c{:016x}", 0.0f64.to_bits()));
                loop_kinds.insert(format!("\0forof-index-{index}"), JitKind::Number);
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
                            || loop_locals.contains_key(name.id.sym.as_ref())
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
                        let element_index = loop_kinds.len();
                        let prefix = match element_kind {
                            JitKind::Number => "ln",
                            JitKind::Boolean => "lb",
                            JitKind::String => "ls",
                            _ => return None,
                        };
                        loop_locals.insert(
                            name.id.sym.to_string(),
                            vec![format!("{prefix}{element_index}")],
                        );
                        loop_kinds.insert(name.id.sym.to_string(), element_kind);
                        if declaration.kind != VarDeclKind::Const {
                            loop_mutable.insert(name.id.sym.to_string());
                        }
                        element_index
                    }
                    ForHead::Pat(pattern) => {
                        let Pat::Ident(name) = pattern.as_ref() else {
                            return None;
                        };
                        if !loop_mutable.contains(name.id.sym.as_ref())
                            || loop_kinds.get(name.id.sym.as_ref())? != &element_kind
                        {
                            return None;
                        }
                        loop_locals
                            .get(name.id.sym.as_ref())?
                            .first()?
                            .get(2..)?
                            .parse::<usize>()
                            .ok()?
                    }
                    ForHead::UsingDecl(_) => return None,
                };
                let mut loop_output = vec!["resultstart".into(), "loop".into()];
                loop_output.extend([
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
                let control_kinds = helper_control_kinds(&loop_kinds, &loop_locals)?;
                let mut early_kinds = None;
                let mut early_values = None;
                encode_aggregate_return_effects(
                    statement.body.as_ref(),
                    requested,
                    parameters,
                    &loop_locals,
                    &loop_mutable,
                    &control_kinds,
                    &loop_kinds,
                    context,
                    &mut early_kinds,
                    &mut early_values,
                    &mut loop_output,
                )?;
                let early_kinds = early_kinds?;
                let early_values = early_values?;
                loop_output.extend([
                    "looptail".into(),
                    index_local,
                    format!("c{:016x}", 1.0f64.to_bits()),
                    "+".into(),
                    format!("setl{index}"),
                    "loopend".into(),
                ]);
                let mut fallback_kinds = loop_kinds.clone();
                let mut fallback_values = std::collections::HashMap::new();
                let mut fallback_output = Vec::new();
                materialize_helper_returns(
                    rest,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut fallback_kinds,
                    &mut fallback_values,
                    &mut fallback_output,
                )?;
                if early_kinds != fallback_kinds || early_values != fallback_values {
                    return None;
                }
                output.extend(loop_output);
                output.extend(fallback_output);
                output.push("resultend".into());
                for (slot, name) in loop_kinds
                    .keys()
                    .filter(|name| !kinds.contains_key(*name))
                    .enumerate()
                {
                    let kind = fallback_kinds.remove(name)?;
                    fallback_kinds.insert(
                        format!("\0forof-result-{slot}-{}", fallback_kinds.len()),
                        kind,
                    );
                }
                *kinds = fallback_kinds;
                *materialized = fallback_values;
                Some(())
            }
            Stmt::ForIn(statement)
                if !rest.is_empty() && contains_aggregate_return(statement.body.as_ref()) =>
            {
                let mut loop_locals = locals.clone();
                let mut loop_mutable = mutable.clone();
                let mut loop_kinds = kinds.clone();
                let mut source = Vec::new();
                encode_expression(
                    statement.right.as_ref(),
                    parameters,
                    &loop_locals,
                    context,
                    &mut source,
                )?;
                if jit_expression_kind(&source)?.0 != JitKind::Dictionary {
                    return None;
                }
                let dictionary = dictionary_prefix(&source)?;
                let local_prefix = format!("{dictionary}l");
                let source_local = if let [source] = source.as_slice() {
                    source
                        .starts_with(&local_prefix)
                        .then(|| source.clone())
                } else {
                    None
                }
                .unwrap_or_else(|| {
                    let source_index = loop_kinds.len();
                    output.extend(source);
                    loop_kinds.insert(
                        format!("\0forin-source-{source_index}"),
                        JitKind::Dictionary,
                    );
                    format!("{dictionary}l{source_index}")
                });
                let index = loop_kinds.len();
                output.push(format!("c{:016x}", 0.0f64.to_bits()));
                loop_kinds.insert(format!("\0forin-index-{index}"), JitKind::Number);
                let index_local = format!("ln{index}");
                let key_index = match &statement.left {
                    ForHead::VarDecl(declaration) => {
                        let [declarator] = declaration.decls.as_slice() else {
                            return None;
                        };
                        let Pat::Ident(name) = &declarator.name else {
                            return None;
                        };
                        if declarator.init.is_some()
                            || parameters.contains_key(name.id.sym.as_ref())
                            || loop_locals.contains_key(name.id.sym.as_ref())
                        {
                            return None;
                        }
                        encode_string("", output)?;
                        let key_index = loop_kinds.len();
                        loop_locals.insert(
                            name.id.sym.to_string(),
                            vec![format!("ls{key_index}")],
                        );
                        loop_kinds.insert(name.id.sym.to_string(), JitKind::String);
                        if declaration.kind != VarDeclKind::Const {
                            loop_mutable.insert(name.id.sym.to_string());
                        }
                        Some(key_index)
                    }
                    ForHead::Pat(pattern) => {
                        let Pat::Ident(name) = pattern.as_ref() else {
                            return None;
                        };
                        if !loop_mutable.contains(name.id.sym.as_ref())
                            || loop_kinds.get(name.id.sym.as_ref())? != &JitKind::String
                        {
                            return None;
                        }
                        Some(loop_local_index(
                            loop_locals.get(name.id.sym.as_ref())?.first()?,
                        )?)
                    }
                    ForHead::UsingDecl(_) => return None,
                };
                let mut loop_output = vec![
                    "resultstart".into(),
                    "loop".into(),
                    index_local.clone(),
                    source_local.clone(),
                    "dlen".into(),
                    "<".into(),
                    "while".into(),
                ];
                if let Some(key_index) = key_index {
                    loop_output.extend([
                        source_local,
                        index_local.clone(),
                        "dkeyat".into(),
                        format!("setl{key_index}"),
                    ]);
                }
                let mut early_kinds = None;
                let mut early_values = None;
                encode_aggregate_return_effects(
                    statement.body.as_ref(),
                    requested,
                    parameters,
                    &loop_locals,
                    &loop_mutable,
                    &loop_kinds,
                    &loop_kinds,
                    context,
                    &mut early_kinds,
                    &mut early_values,
                    &mut loop_output,
                )?;
                let early_kinds = early_kinds?;
                let early_values = early_values?;
                loop_output.extend([
                    "looptail".into(),
                    index_local,
                    format!("c{:016x}", 1.0f64.to_bits()),
                    "+".into(),
                    format!("setl{index}"),
                    "loopend".into(),
                ]);
                let mut fallback_kinds = loop_kinds.clone();
                let mut fallback_values = std::collections::HashMap::new();
                let mut fallback_output = Vec::new();
                materialize_helper_returns(
                    rest,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut fallback_kinds,
                    &mut fallback_values,
                    &mut fallback_output,
                )?;
                if early_kinds != fallback_kinds || early_values != fallback_values {
                    return None;
                }
                output.extend(loop_output);
                output.extend(fallback_output);
                output.push("resultend".into());
                for (slot, name) in loop_kinds
                    .keys()
                    .filter(|name| !kinds.contains_key(*name))
                    .enumerate()
                {
                    let kind = fallback_kinds.remove(name)?;
                    fallback_kinds.insert(
                        format!("\0forin-result-{slot}-{}", fallback_kinds.len()),
                        kind,
                    );
                }
                *kinds = fallback_kinds;
                *materialized = fallback_values;
                Some(())
            }
            Stmt::If(branch) => {
                if let Some((name, tag_index, value_index, selected, alternate, probe)) =
                    dynamic_catch_narrowing(branch.test.as_ref(), locals)
                {
                    let consequent = match branch.cons.as_ref() {
                        Stmt::Block(block) => block.stmts.iter().collect::<Vec<_>>(),
                        statement => vec![statement],
                    };
                    let alternate_statements = if let Some(statement) = branch.alt.as_deref() {
                        match statement {
                            Stmt::Block(block) => block.stmts.iter().collect::<Vec<_>>(),
                            statement => vec![statement],
                        }
                    } else {
                        rest.to_vec()
                    };
                    let mut consequent_locals = locals.clone();
                    consequent_locals.insert(
                        name.clone(),
                        vec![format!("{}{value_index}", selected.prefix)],
                    );
                    let mut consequent_kinds = kinds.clone();
                    consequent_kinds.insert(name.clone(), selected.kind);
                    let mut consequent_values = std::collections::HashMap::new();
                    let mut consequent_output = Vec::new();
                    materialize_helper_returns(
                        &consequent,
                        requested,
                        parameters,
                        &consequent_locals,
                        mutable,
                        context,
                        &mut consequent_kinds,
                        &mut consequent_values,
                        &mut consequent_output,
                    )?;
                    let alternate = alternate?;
                    let mut alternate_locals = locals.clone();
                    alternate_locals.insert(
                        name.clone(),
                        vec![format!("{}{value_index}", alternate.prefix)],
                    );
                    let mut alternate_kinds = kinds.clone();
                    alternate_kinds.insert(name, alternate.kind);
                    let mut alternate_values = std::collections::HashMap::new();
                    let mut alternate_output = Vec::new();
                    materialize_helper_returns(
                        &alternate_statements,
                        requested,
                        parameters,
                        &alternate_locals,
                        mutable,
                        context,
                        &mut alternate_kinds,
                        &mut alternate_values,
                        &mut alternate_output,
                    )?;
                    if consequent_values != alternate_values
                        || consequent_kinds.len() != alternate_kinds.len()
                    {
                        return None;
                    }
                    output.extend([
                        format!("ln{tag_index}"),
                        format!(
                            "c{:016x}",
                            f64::from(caught_throw_tag(&selected)?).to_bits()
                        ),
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
                    output.push("if".into());
                    output.extend(consequent_output);
                    output.push("else".into());
                    output.extend(alternate_output);
                    output.push("end".into());
                    *kinds = consequent_kinds;
                    *materialized = consequent_values;
                    return Some(());
                }
                if branch.alt.is_some() && !rest.is_empty() {
                    let control_kinds = helper_control_kinds(kinds, locals)?;
                    encode_loop_effects(
                        statement,
                        parameters,
                        locals,
                        mutable,
                        (&control_kinds, root_loop_control(), &[]),
                        context,
                        output,
                    )?;
                    return materialize_helper_returns(
                        rest,
                        requested,
                        parameters,
                        locals,
                        mutable,
                        context,
                        kinds,
                        materialized,
                        output,
                    );
                }
                let consequent = match branch.cons.as_ref() {
                    Stmt::Block(block) => block.stmts.iter().collect::<Vec<_>>(),
                    statement => vec![statement],
                };
                let alternate = if let Some(alternate) = branch.alt.as_deref() {
                    match alternate {
                        Stmt::Block(block) => block.stmts.iter().collect::<Vec<_>>(),
                        statement => vec![statement],
                    }
                } else {
                    rest.to_vec()
                };
                let mut consequent_kinds = kinds.clone();
                let mut consequent_values = std::collections::HashMap::new();
                let mut consequent_output = Vec::new();
                materialize_helper_returns(
                    &consequent,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut consequent_kinds,
                    &mut consequent_values,
                    &mut consequent_output,
                )?;
                let mut alternate_kinds = kinds.clone();
                let mut alternate_values = std::collections::HashMap::new();
                let mut alternate_output = Vec::new();
                materialize_helper_returns(
                    &alternate,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    &mut alternate_kinds,
                    &mut alternate_values,
                    &mut alternate_output,
                )?;
                if consequent_kinds != alternate_kinds
                    || consequent_values != alternate_values
                {
                    return None;
                }
                encode_condition(branch.test.as_ref(), parameters, locals, context, output)?;
                output.push("if".into());
                output.extend(consequent_output);
                output.push("else".into());
                output.extend(alternate_output);
                output.push("end".into());
                *kinds = consequent_kinds;
                *materialized = consequent_values;
                Some(())
            }
            Stmt::Try(statement)
                if rest.is_empty()
                    && statement.handler.is_some()
                    && statement.finalizer.as_ref().is_none_or(|finalizer| {
                        finalizer
                            .stmts
                            .iter()
                            .all(|statement| matches!(statement, Stmt::Expr(_)))
                    }) =>
            {
                let handler = statement.handler.as_ref()?;
                let (Stmt::Return(returned), control) = statement.block.stmts.split_last()? else {
                    return None;
                };
                let mut caught = Vec::new();
                for statement in &statement.block.stmts {
                    collect_caught_throw_kind(
                        statement,
                        parameters,
                        locals,
                        context,
                        &mut caught,
                    )?;
                }
                if caught.is_empty() {
                    return None;
                }
                let tagged = caught.len() > 1;
                output.push(if tagged { "trystarttag" } else { "trystart" }.into());
                let caught_control = LoopControl {
                    catch_active: true,
                    catch_tagged: tagged,
                    ..root_loop_control()
                };
                let control_kinds = helper_control_kinds(kinds, locals)?;
                for statement in control {
                    encode_loop_effects(
                        statement,
                        parameters,
                        locals,
                        mutable,
                        (&control_kinds, caught_control, &[]),
                        context,
                        output,
                    )?;
                }
                let placeholder = kinds.len();
                if tagged {
                    output.extend([
                        "c0000000000000000".into(),
                        "c0000000000000000".into(),
                        "tagnum".into(),
                    ]);
                } else {
                    let caught = &caught[0];
                    match caught.kind {
                        JitKind::Number => output.push("c0000000000000000".into()),
                        JitKind::Boolean => {
                            output.extend(["c0000000000000000".into(), "asbool".into()]);
                        }
                        JitKind::String => output.push("t".into()),
                        JitKind::Array => output.push("arrayempty".into()),
                        JitKind::Dictionary => output.push(
                            match caught.prefix {
                                "dnl" => "dnempty",
                                "dbl" => "dbempty",
                                "dsl" => "dsempty",
                                _ => return None,
                            }
                            .into(),
                        ),
                        JitKind::Dynamic => return None,
                    }
                }
                let mut normal_kinds = kinds.clone();
                normal_kinds.insert(
                    format!("\0catch-placeholder-{placeholder}"),
                    if tagged {
                        JitKind::Number
                    } else {
                        caught[0].kind
                    },
                );
                if tagged {
                    normal_kinds.insert(
                        format!("\0catch-placeholder-{}", placeholder + 1),
                        JitKind::Dynamic,
                    );
                }
                let mut normal_values = std::collections::HashMap::new();
                materialize_fixed_literal(
                    returned.arg.as_deref()?,
                    "",
                    requested,
                    parameters,
                    locals,
                    context,
                    &mut normal_kinds,
                    &mut normal_values,
                    output,
                )?;
                output.push("catch".into());
                let mut catch_locals = locals.clone();
                let mut catch_kinds = kinds.clone();
                if let Some(parameter) = &handler.param {
                    let Pat::Ident(parameter) = parameter else {
                        return None;
                    };
                    if tagged {
                        let variants = caught
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
                            parameter.id.sym.to_string(),
                            vec![format!("x{placeholder}:{}:{variants}", placeholder + 1)],
                        );
                        catch_kinds.insert(
                            format!("\0catch-tag-{placeholder}"),
                            JitKind::Number,
                        );
                        catch_kinds.insert(parameter.id.sym.to_string(), JitKind::Number);
                    } else {
                        catch_locals.insert(
                            parameter.id.sym.to_string(),
                            vec![format!("{}{placeholder}", caught[0].prefix)],
                        );
                        catch_kinds.insert(parameter.id.sym.to_string(), caught[0].kind);
                    }
                } else {
                    for slot in 0..if tagged { 2 } else { 1 } {
                        catch_kinds.insert(
                            format!("\0catch-{}-{slot}", catch_kinds.len()),
                            JitKind::Number,
                        );
                    }
                }
                let catch_body = handler.body.stmts.iter().collect::<Vec<_>>();
                let mut catch_values = std::collections::HashMap::new();
                let mut catch_output = Vec::new();
                materialize_helper_returns(
                    &catch_body,
                    requested,
                    parameters,
                    &catch_locals,
                    mutable,
                    context,
                    &mut catch_kinds,
                    &mut catch_values,
                    &mut catch_output,
                )?;
                if normal_values != catch_values || normal_kinds.len() != catch_kinds.len() {
                    return None;
                }
                output.extend(catch_output);
                output.push("tryend".into());
                if let Some(finalizer) = &statement.finalizer {
                    let control_kinds = helper_control_kinds(&normal_kinds, locals)?;
                    encode_loop_effects(
                        &Stmt::Block(finalizer.clone()),
                        parameters,
                        locals,
                        mutable,
                        (&control_kinds, root_loop_control(), &[]),
                        context,
                        output,
                    )?;
                }
                *kinds = normal_kinds;
                *materialized = normal_values;
                Some(())
            }
            Stmt::Try(statement)
                if rest.is_empty()
                    && statement.handler.is_none()
                    && statement.finalizer.as_ref().is_some_and(|finalizer| {
                        finalizer
                            .stmts
                            .iter()
                            .all(|statement| matches!(statement, Stmt::Expr(_)))
                    }) =>
            {
                let body = statement.block.stmts.iter().collect::<Vec<_>>();
                materialize_helper_returns(
                    &body,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    kinds,
                    materialized,
                    output,
                )?;
                let control_kinds = helper_control_kinds(kinds, locals)?;
                encode_loop_effects(
                    &Stmt::Block(statement.finalizer.clone()?),
                    parameters,
                    locals,
                    mutable,
                    (&control_kinds, root_loop_control(), &[]),
                    context,
                    output,
                )
            }
            Stmt::Switch(switch) if rest.is_empty() => {
                if !matches!(switch.cases.last(), Some(case) if case.test.is_none()) {
                    return None;
                }
                let mut discriminant = Vec::new();
                encode_expression(
                    switch.discriminant.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut discriminant,
                )?;
                let discriminant = materialize_helper_value(discriminant, kinds, output)?;
                materialize_helper_switch_cases(
                    &switch.cases,
                    &discriminant,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    kinds,
                    materialized,
                    output,
                )
            }
            _ => {
                let control_kinds = helper_control_kinds(kinds, locals)?;
                encode_loop_effects(
                    statement,
                    parameters,
                    locals,
                    mutable,
                    (&control_kinds, root_loop_control(), &[]),
                    context,
                    output,
                )?;
                materialize_helper_returns(
                    rest,
                    requested,
                    parameters,
                    locals,
                    mutable,
                    context,
                    kinds,
                    materialized,
                    output,
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_fixed_call(
        call: &CallExpr,
        requested: &std::collections::HashSet<String>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        materialized: &mut std::collections::HashMap<String, Vec<String>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let callable = resolve_callable(callee.as_ref(), context.helpers)?;
        let (helper_parameters, steps, body) = callable_parts(callable)?;
        if helper_parameters.len() != call.args.len() {
            return None;
        }
        let mut helper_locals = context.module_locals.clone();
        let mut helper_mutable = std::collections::HashSet::new();
        for (index, (parameter, argument)) in helper_parameters.iter().zip(&call.args).enumerate() {
            let Pat::Ident(parameter) = parameter else {
                return None;
            };
            let path = format!(".argument{index}");
            let argument_paths = std::collections::HashSet::from([path.clone()]);
            let mut argument_values = std::collections::HashMap::new();
            materialize_fixed_literal(
                argument.expr.as_ref(),
                &path,
                &argument_paths,
                parameters,
                locals,
                context,
                kinds,
                &mut argument_values,
                output,
            )?;
            helper_locals.insert(
                parameter.id.sym.to_string(),
                argument_values.remove(&path)?,
            );
            helper_mutable.insert(parameter.id.sym.to_string());
        }
        let active = match callee.as_ref() {
            Expr::Ident(name) => Some(name.sym.to_string()),
            _ => None,
        };
        if let Some(name) = &active {
            if context.active.contains(name) {
                return None;
            }
            context.active.push(name.clone());
        }
        let result = (|| {
            let no_parameters = std::collections::HashMap::new();
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    LocalStep::Declare {
                        name,
                        initializer,
                        mutable,
                    } => {
                        let path = format!(".local{index}");
                        let local_paths = std::collections::HashSet::from([path.clone()]);
                        let mut local_values = std::collections::HashMap::new();
                        materialize_fixed_literal(
                            initializer,
                            &path,
                            &local_paths,
                            &no_parameters,
                            &helper_locals,
                            context,
                            kinds,
                            &mut local_values,
                            output,
                        )?;
                        helper_locals
                            .insert(name.sym.to_string(), local_values.remove(&path)?);
                        if mutable {
                            helper_mutable.insert(name.sym.to_string());
                        }
                    }
                    LocalStep::Assign {
                        name,
                        operation,
                        value,
                    } => {
                        if !helper_mutable.contains(name.sym.as_ref()) {
                            return None;
                        }
                        let current = helper_locals.get(name.sym.as_ref())?.clone();
                        let target = loop_local_index(current.first()?)?;
                        let expected = jit_expression_kind(&current)?.0;
                        let mut encoded = Vec::new();
                        if operation == AssignOp::AddAssign {
                            let mut right = Vec::new();
                            encode_expression(
                                value,
                                &no_parameters,
                                &helper_locals,
                                context,
                                &mut right,
                            )?;
                            append_add(current, right, &mut encoded)?;
                        } else {
                            if operation != AssignOp::Assign {
                                encoded.extend(current);
                            }
                            encode_expression(
                                value,
                                &no_parameters,
                                &helper_locals,
                                context,
                                &mut encoded,
                            )?;
                            if operation != AssignOp::Assign {
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
                        }
                        if jit_expression_kind(&encoded)?.0 != expected {
                            return None;
                        }
                        output.extend(encoded);
                        if expected == JitKind::Boolean {
                            output.push("asbool".into());
                        } else if expected == JitKind::Array {
                            output.push("arrayhandle".into());
                        }
                        output.push(format!("setl{target}"));
                    }
                    LocalStep::Update { name, operation } => {
                        if !helper_mutable.contains(name.sym.as_ref()) {
                            return None;
                        }
                        let current = helper_locals.get(name.sym.as_ref())?.clone();
                        if jit_expression_kind(&current)?.0 != JitKind::Number {
                            return None;
                        }
                        let target = loop_local_index(current.first()?)?;
                        output.extend(current);
                        output.extend([
                            format!("c{:016x}", 1.0f64.to_bits()),
                            match operation {
                                UpdateOp::PlusPlus => "+".into(),
                                UpdateOp::MinusMinus => "-".into(),
                            },
                            format!("setl{target}"),
                        ]);
                    }
                    LocalStep::Effect(expression) => {
                        encode_expression(
                            expression,
                            &no_parameters,
                            &helper_locals,
                            context,
                            output,
                        )?;
                        output.push("drop".into());
                    }
                    LocalStep::DestructureArray {
                        bindings,
                        rest,
                        initializer,
                        mutable,
                        assign_existing,
                    } => {
                        let names = bindings
                            .iter()
                            .map(|(_, name, _)| *name)
                            .chain(rest.iter().map(|(_, name)| *name));
                        if names.clone().any(|name| {
                            assign_existing && !helper_mutable.contains(name.sym.as_ref())
                        }) {
                            return None;
                        }
                        let mut source = Vec::new();
                        encode_expression(
                            initializer,
                            &no_parameters,
                            &helper_locals,
                            context,
                            &mut source,
                        )?;
                        if source.len() == 1 {
                            if let Some(untag) = jit_typed_array_union_untag(&source[0]) {
                                source.push(untag.into());
                            }
                        }
                        let prefix = array_prefix(&source)?;
                        let source = materialize_helper_value(source, kinds, output)?;
                        for (element, name, default) in bindings {
                            let mut value = source.clone();
                            value.extend([
                                format!("c{:016x}", (element as f64).to_bits()),
                                format!("{prefix}get"),
                            ]);
                            if let Some(default) = default {
                                let expected = jit_expression_kind(&value)?.0;
                                let mut fallback = Vec::new();
                                encode_expression(
                                    default,
                                    &no_parameters,
                                    &helper_locals,
                                    context,
                                    &mut fallback,
                                )?;
                                if expected == JitKind::Boolean {
                                    let mut boolean = Vec::new();
                                    append_boolean(fallback, &mut boolean)?;
                                    fallback = boolean;
                                }
                                if jit_expression_kind(&fallback)?.0 != expected {
                                    return None;
                                }
                                value.push("ifpresent".into());
                                value.push("else".into());
                                value.extend(fallback);
                                value.push("end".into());
                            }
                            if assign_existing {
                                let current = helper_locals.get(name.sym.as_ref())?;
                                if jit_expression_kind(current)?.0
                                    != jit_expression_kind(&value)?.0
                                {
                                    return None;
                                }
                                let target = loop_local_index(current.first()?)?;
                                output.extend(value);
                                output.push(format!("setl{target}"));
                            } else {
                                let value = materialize_helper_value(value, kinds, output)?;
                                helper_locals.insert(name.sym.to_string(), value);
                                if mutable {
                                    helper_mutable.insert(name.sym.to_string());
                                }
                            }
                        }
                        if let Some((start, name)) = rest {
                            let mut value = source;
                            value.extend([
                                format!("c{:016x}", (start as f64).to_bits()),
                                format!("c{:016x}", f64::INFINITY.to_bits()),
                                "arrayslice".into(),
                            ]);
                            if assign_existing {
                                let current = helper_locals.get(name.sym.as_ref())?;
                                if jit_expression_kind(current)?.0 != JitKind::Array {
                                    return None;
                                }
                                let target = loop_local_index(current.first()?)?;
                                output.extend(value);
                                output.extend(["arrayhandle".into(), format!("setl{target}")]);
                            } else {
                                let value = materialize_helper_value(value, kinds, output)?;
                                helper_locals.insert(name.sym.to_string(), value);
                                if mutable {
                                    helper_mutable.insert(name.sym.to_string());
                                }
                            }
                        }
                    }
                    LocalStep::DestructureObject {
                        bindings,
                        initializer,
                        mutable,
                        assign_existing,
                    } => {
                        if bindings.iter().any(|(_, name, _)| {
                            assign_existing && !helper_mutable.contains(name.sym.as_ref())
                        }) {
                            return None;
                        }
                        let requested = bindings
                            .iter()
                            .map(|(path, _, _)| path.clone())
                            .collect();
                        let mut values = std::collections::HashMap::new();
                        if let Expr::Call(call) = initializer {
                            materialize_fixed_call(
                                call,
                                &requested,
                                &no_parameters,
                                &helper_locals,
                                context,
                                kinds,
                                &mut values,
                                output,
                            )?;
                        } else {
                            materialize_fixed_literal(
                                initializer,
                                "",
                                &requested,
                                &no_parameters,
                                &helper_locals,
                                context,
                                kinds,
                                &mut values,
                                output,
                            )?;
                        }
                        for (path, name, default) in bindings {
                            let value = if let Some(value) = values.remove(&path) {
                                value
                            } else {
                                let default = default?;
                                let mut encoded = Vec::new();
                                encode_expression(
                                    default,
                                    &no_parameters,
                                    &helper_locals,
                                    context,
                                    &mut encoded,
                                )?;
                                materialize_helper_value(encoded, kinds, output)?
                            };
                            if assign_existing {
                                let current = helper_locals.get(name.sym.as_ref())?;
                                if jit_expression_kind(current)?.0
                                    != jit_expression_kind(&value)?.0
                                {
                                    return None;
                                }
                                let target = loop_local_index(current.first()?)?;
                                output.extend(value);
                                output.push(format!("setl{target}"));
                            } else {
                                helper_locals.insert(name.sym.to_string(), value);
                                if mutable {
                                    helper_mutable.insert(name.sym.to_string());
                                }
                            }
                        }
                    }
                }
            }
            match body {
                NumericBody::Expression(expression) => materialize_fixed_literal(
                    expression,
                    "",
                    requested,
                    &no_parameters,
                    &helper_locals,
                    context,
                    kinds,
                    materialized,
                    output,
                ),
                NumericBody::Statements(statements) => {
                    let statements = statements.iter().collect::<Vec<_>>();
                    materialize_helper_returns(
                        &statements,
                        requested,
                        &no_parameters,
                        &helper_locals,
                        &helper_mutable,
                        context,
                        kinds,
                        materialized,
                        output,
                    )
                }
            }
        })();
        if active.is_some() {
            context.active.pop();
        }
        result
    }

    fn collect_fixed_object_pattern<'a>(
        pattern: &'a thaw_parser::ast::ObjectPat,
        path: &str,
        bindings: &mut Vec<(String, &'a Ident, Option<&'a Expr>)>,
    ) -> Option<()> {
        for property in &pattern.props {
            match property {
                ObjectPatProp::Assign(property) => bindings.push((
                    format!("{path}.{}", property.key.sym),
                    &property.key.id,
                    property.value.as_deref(),
                )),
                ObjectPatProp::KeyValue(property) => collect_fixed_object_bindings(
                    property.value.as_ref(),
                    &format!(
                        "{path}.{}",
                        destructuring_property_name(&property.key)?
                    ),
                    bindings,
                )?,
                ObjectPatProp::Rest(_) => return None,
            }
        }
        Some(())
    }

    fn split_numeric_body(statements: &[Stmt]) -> Option<(Vec<LocalStep<'_>>, NumericBody<'_>)> {
        type ArrayBindings<'a> = (
            Vec<(usize, &'a Ident, Option<&'a Expr>)>,
            Option<(usize, &'a Ident)>,
        );

        fn collect_array_bindings(pattern: &thaw_parser::ast::ArrayPat) -> Option<ArrayBindings<'_>> {
            let mut bindings = Vec::new();
            let mut rest = None;
            for (index, element) in pattern.elems.iter().enumerate() {
                let Some(element) = element else {
                    continue;
                };
                match element {
                    Pat::Ident(name) => bindings.push((index, &name.id, None)),
                    Pat::Assign(assignment) => {
                        let Pat::Ident(name) = assignment.left.as_ref() else {
                            return None;
                        };
                        bindings.push((index, &name.id, Some(assignment.right.as_ref())));
                    }
                    Pat::Rest(element) => {
                        let Pat::Ident(name) = element.arg.as_ref() else {
                            return None;
                        };
                        if rest.replace((index, &name.id)).is_some()
                            || index + 1 != pattern.elems.len()
                        {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            (!bindings.is_empty() || rest.is_some()).then_some((bindings, rest))
        }

        let mut steps = Vec::new();
        let mut offset = 0;
        loop {
            match statements.get(offset) {
                Some(Stmt::Decl(Decl::Var(declaration))) => {
                    for declarator in &declaration.decls {
                        match &declarator.name {
                            Pat::Ident(name) => steps.push(LocalStep::Declare {
                                name: &name.id,
                                initializer: declarator.init.as_deref()?,
                                mutable: declaration.kind != VarDeclKind::Const,
                            }),
                            Pat::Array(pattern) => {
                                if let Some((bindings, rest)) = collect_array_bindings(pattern) {
                                    steps.push(LocalStep::DestructureArray {
                                        bindings,
                                        rest,
                                        initializer: declarator.init.as_deref()?,
                                        mutable: declaration.kind != VarDeclKind::Const,
                                        assign_existing: false,
                                    });
                                } else {
                                    let mut bindings = Vec::new();
                                    collect_fixed_array_pattern(pattern, "", &mut bindings)?;
                                    if bindings.is_empty() {
                                        return None;
                                    }
                                    steps.push(LocalStep::DestructureObject {
                                        bindings,
                                        initializer: declarator.init.as_deref()?,
                                        mutable: declaration.kind != VarDeclKind::Const,
                                        assign_existing: false,
                                    });
                                }
                            }
                            Pat::Object(_) => {
                                let mut bindings = Vec::new();
                                collect_fixed_object_bindings(
                                    &declarator.name,
                                    "",
                                    &mut bindings,
                                )?;
                                if bindings.is_empty() {
                                    return None;
                                }
                                steps.push(LocalStep::DestructureObject {
                                    bindings,
                                    initializer: declarator.init.as_deref()?,
                                    mutable: declaration.kind != VarDeclKind::Const,
                                    assign_existing: false,
                                });
                            }
                            _ => return None,
                        }
                    }
                }
                Some(Stmt::Expr(statement)) => {
                    let mut expression = statement.expr.as_ref();
                    while let Expr::Paren(parenthesized) = expression {
                        expression = parenthesized.expr.as_ref();
                    }
                    match expression {
                    Expr::Assign(assignment) => {
                        match &assignment.left {
                            AssignTarget::Simple(SimpleAssignTarget::Ident(name)) => {
                                steps.push(LocalStep::Assign {
                                    name: &name.id,
                                    operation: assignment.op,
                                    value: assignment.right.as_ref(),
                                });
                            }
                            AssignTarget::Pat(thaw_parser::ast::AssignTargetPat::Array(pattern))
                                if assignment.op == AssignOp::Assign =>
                            {
                                if let Some((bindings, rest)) = collect_array_bindings(pattern) {
                                    steps.push(LocalStep::DestructureArray {
                                        bindings,
                                        rest,
                                        initializer: assignment.right.as_ref(),
                                        mutable: true,
                                        assign_existing: true,
                                    });
                                } else {
                                    let mut bindings = Vec::new();
                                    collect_fixed_array_pattern(pattern, "", &mut bindings)?;
                                    if bindings.is_empty() {
                                        return None;
                                    }
                                    steps.push(LocalStep::DestructureObject {
                                        bindings,
                                        initializer: assignment.right.as_ref(),
                                        mutable: true,
                                        assign_existing: true,
                                    });
                                }
                            }
                            AssignTarget::Pat(thaw_parser::ast::AssignTargetPat::Object(pattern))
                                if assignment.op == AssignOp::Assign =>
                            {
                                let mut bindings = Vec::new();
                                collect_fixed_object_pattern(pattern, "", &mut bindings)?;
                                if bindings.is_empty() {
                                    return None;
                                }
                                steps.push(LocalStep::DestructureObject {
                                    bindings,
                                    initializer: assignment.right.as_ref(),
                                    mutable: true,
                                    assign_existing: true,
                                });
                            }
                            _ => steps.push(LocalStep::Effect(statement.expr.as_ref())),
                        }
                    }
                    Expr::Update(update) => {
                        if let Expr::Ident(name) = update.arg.as_ref() {
                            steps.push(LocalStep::Update {
                                name,
                                operation: update.op,
                            });
                        } else {
                            steps.push(LocalStep::Effect(statement.expr.as_ref()));
                        }
                    }
                    _ => steps.push(LocalStep::Effect(statement.expr.as_ref())),
                }
                }
                _ => break,
            }
            offset += 1;
        }
        (!statements[offset..].is_empty())
            .then_some((steps, NumericBody::Statements(&statements[offset..])))
    }
    };
}
