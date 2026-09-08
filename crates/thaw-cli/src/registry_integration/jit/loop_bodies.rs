macro_rules! jit_loop_bodies {
    () => {
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

    };
}

