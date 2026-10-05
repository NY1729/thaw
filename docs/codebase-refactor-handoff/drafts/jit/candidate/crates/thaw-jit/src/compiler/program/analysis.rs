impl NumericProgram {

    fn callback_source_kind(&self, index: usize) -> Option<CallbackSourceKind> {
        self.1.get(index).copied().flatten()
    }

    fn required_args(&self) -> usize {
        self.0
            .iter()
            .filter_map(|value| match value {
                NumericValue::Argument(index) => Some(*index as usize + 1),
                NumericValue::DynamicArgument(index) => Some(*index as usize + 2),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    fn callback_result_kind(&self) -> Option<CallbackValueKind> {
        self.callback_result_kind_for_environment(&[], &[], false)
    }

    fn callback_result_kind_for_environment(
        &self,
        argument_kinds: &[u8],
        dynamic_pair_starts: &[u8],
        validate_environment: bool,
    ) -> Option<CallbackValueKind> {
        let mut stack: Vec<Option<CallbackValueKind>> = Vec::new();
        let mut locals = [CallbackLocalState::Uninitialized; 8];
        let mut branches: Vec<CallbackBranchState> = Vec::new();
        let mut guards: Vec<CallbackBranchState> = Vec::new();
        let mut results: Vec<CallbackResultState> = Vec::new();
        let mut switches: Vec<CallbackSwitchState> = Vec::new();
        let mut loops: Vec<CallbackLoopState> = Vec::new();
        let mut tries: Vec<CallbackTryState> = Vec::new();
        let mut catches: Vec<CallbackCatchState> = Vec::new();
        let mut early_returns = Vec::new();
        let mut reachable = true;
        let mut index = 0;

        while index < self.0.len() {
            let value = &self.0[index];
            if !reachable && !is_callback_control(value) {
                index += 1;
                continue;
            }
            let source = self.callback_source_kind(index);
            if reachable {
                locals = callback_locals_from_stack(&stack);
            }
            match value {
                NumericValue::Argument(argument) => {
                    let declared = source.and_then(CallbackSourceKind::representation);
                    let actual = validate_environment
                        .then(|| argument_kinds.get(usize::from(*argument)).copied())
                        .flatten()
                        .and_then(callback_value_kind_from_code);
                    if let (Some(declared), Some(actual)) = (declared, actual) {
                        if declared != actual && present_callback_kind(actual) != Some(declared) {
                            return None;
                        }
                    }
                    stack.push(if validate_environment {
                        actual
                    } else {
                        declared
                    });
                }
                NumericValue::DynamicArgument(argument) => {
                    if validate_environment
                        && !dynamic_pair_starts.contains(argument)
                    {
                        return None;
                    }
                    stack.push(Some(CallbackValueKind::Dynamic));
                }
                NumericValue::ObjectField(_, _)
                | NumericValue::OptionalObjectField(_, _)
                | NumericValue::OptionalTupleField(_, _)
                | NumericValue::NullishObjectField(_, _)
                | NumericValue::NullishTupleField(_, _) => {
                    stack.push(source.and_then(CallbackSourceKind::representation));
                }
                NumericValue::Constant(_) | NumericValue::StringConstant(_) => {
                    stack.push(source.and_then(CallbackSourceKind::representation));
                }
                NumericValue::LocalGet(local) => {
                    let local = usize::from(*local);
                    let actual = *stack.get(local)?;
                    let Some(actual_kind) = actual else { return None; };
                    if let Some(CallbackSourceKind::Local(declared)) = source {
                        if declared != actual_kind && present_callback_kind(actual_kind) != Some(declared) {
                            return None;
                        }
                    }
                    stack.push(Some(actual_kind));
                }
                NumericValue::LocalSet(local) => {
                    let local = usize::from(*local);
                    let value = stack.pop()?;
                    let slot = stack.get_mut(local)?;
                    *slot = value;
                }
                NumericValue::Drop => {
                    stack.pop()?;
                }
                NumericValue::Duplicate => stack.push(*stack.last()?),
                NumericValue::DropUnder => {
                    let top = stack.pop()?;
                    stack.pop()?;
                    stack.push(top);
                }
                NumericValue::DuplicatePair => {
                    let pair = [*stack.get(stack.len().checked_sub(2)?)?, *stack.last()?];
                    stack.extend(pair);
                }
                NumericValue::Select => {
                    let alternate = stack.pop()?;
                    let consequent = stack.pop()?;
                    stack.pop()?;
                    stack.push(join_callback_kind(consequent, alternate));
                }
                NumericValue::ConditionalStart => {
                    if reachable { stack.pop()?; }
                    branches.push(CallbackBranchState {
                        base_stack: stack.clone(),
                        base_locals: locals,
                        base_reachable: reachable,
                        first_stack: None,
                        first_locals: None,
                        first_reachable: false,
                    });
                }
                NumericValue::PresentConditionalStart => {
                    let mut present_stack = stack.clone();
                    let mut base_stack = stack.clone();
                    if reachable {
                        let top = present_stack.last_mut()?;
                        *top = (*top).and_then(present_callback_kind);
                        base_stack.pop()?;
                    }
                    branches.push(CallbackBranchState {
                        base_stack,
                        base_locals: locals,
                        base_reachable: reachable,
                        first_stack: None,
                        first_locals: None,
                        first_reachable: false,
                    });
                    stack = present_stack;
                }
                NumericValue::GuardStart => {
                    if reachable { stack.pop()?; }
                    guards.push(CallbackBranchState {
                        base_stack: stack.clone(),
                        base_locals: locals,
                        base_reachable: reachable,
                        first_stack: None,
                        first_locals: None,
                        first_reachable: false,
                    });
                }
                NumericValue::GuardAlternate => {
                    let guard = guards.last_mut()?;
                    if reachable {
                        guard.first_stack = Some(stack.clone());
                        guard.first_locals = Some(locals);
                        guard.first_reachable = true;
                    }
                    stack = guard.base_stack.clone();
                    locals = guard.base_locals;
                    reachable = guard.base_reachable;
                }
                NumericValue::GuardEnd => {
                    let guard = guards.pop()?;
                    match (guard.first_reachable, reachable) {
                        (true, true) => {
                            stack = join_callback_stacks(guard.first_stack.as_deref()?, &stack)?;
                            locals = join_callback_locals(guard.first_locals?, locals);
                        }
                        (true, false) => { stack = guard.first_stack?; locals = guard.first_locals?; reachable = true; }
                        (false, true) => {}
                        (false, false) => { stack = guard.base_stack; locals = guard.base_locals; }
                    }
                }
                NumericValue::ConditionalAlternate => {
                    let branch = branches.last_mut()?;
                    if reachable {
                        branch.first_stack = Some(stack.clone());
                        branch.first_locals = Some(locals);
                        branch.first_reachable = true;
                    }
                    stack = branch.base_stack.clone();
                    locals = branch.base_locals;
                    reachable = branch.base_reachable;
                }
                NumericValue::ShortCircuit(_) => {
                    let mut short_stack = stack.clone();
                    if reachable {
                        stack.pop()?;
                        let value = stack.pop()?;
                        short_stack = stack.clone();
                        short_stack.push(value);
                    }
                    branches.push(CallbackBranchState {
                        base_stack: stack.clone(),
                        base_locals: locals,
                        base_reachable: reachable,
                        first_stack: reachable.then_some(short_stack),
                        first_locals: reachable.then_some(locals),
                        first_reachable: reachable,
                    });
                }
                NumericValue::ShortCircuitEnd => {
                    let branch = branches.pop()?;
                    match (branch.first_reachable, reachable) {
                        (true, true) => {
                            stack = join_callback_stacks(branch.first_stack.as_deref()?, &stack)?;
                            locals = join_callback_locals(branch.first_locals?, locals);
                        }
                        (true, false) => { stack = branch.first_stack?; locals = branch.first_locals?; reachable = true; }
                        (false, true) => {}
                        (false, false) => { stack = branch.base_stack; locals = branch.base_locals; }
                    }
                }
                NumericValue::ResultStart => results.push(CallbackResultState {
                    base_stack: stack.clone(),
                    returns: Vec::new(),
                }),
                NumericValue::ResultReturn(count) => {
                    if !reachable { index += 1; continue; }
                    let state = results.last_mut()?;
                    let available = stack.len().checked_sub(state.base_stack.len())?;
                    let count = if *count == 0 {
                        available
                    } else {
                        usize::from(*count)
                    };
                    if count == 0 || count > available {
                        return None;
                    }
                    let mut exit = stack[..state.base_stack.len()].to_vec();
                    exit.extend_from_slice(&stack[stack.len() - count..]);
                    state.returns.push(exit);
                    stack = state.base_stack.clone();
                    reachable = false;
                }
                NumericValue::ResultEnd => {
                    let state = results.pop()?;
                    let mut exits = state.returns;
                    if reachable { exits.push(stack.clone()); }
                    if exits.is_empty() { reachable = false; }
                    else { stack = join_callback_exits(&exits)?; reachable = true; }
                }
                NumericValue::SwitchStart => {
                    let snapshot = reachable.then(|| {
                        Box::new(CallbackAnalyzerSnapshot {
                            stack: stack.clone(),
                            locals,
                            reachable,
                            branches: branches.clone(),
                            guards: guards.clone(),
                            results: results.clone(),
                            switches: switches.clone(),
                            loops: loops.clone(),
                            tries: tries.clone(),
                            catches: catches.clone(),
                            early_returns: early_returns.clone(),
                        })
                    });
                    switches.push(CallbackSwitchState {
                        start_index: index,
                        phase: if reachable { SwitchAnalysisPhase::Discover } else { SwitchAnalysisPhase::Suppressed },
                        snapshot,
                        default_seed: None,
                        base_stack: stack.clone(),
                        base_locals: locals,
                        dispatch_stack: stack.clone(),
                        dispatch_reachable: reachable,
                        exits: Vec::new(),
                        pending_fallthrough: None,
                        active_body: false,
                        has_default: false,
                    });
                }
                NumericValue::SwitchCaseStart => {
                    let switch = switches.last_mut()?;
                    if switch.phase == SwitchAnalysisPhase::Suppressed {
                        reachable = false;
                        switch.active_body = false;
                        index += 1;
                        continue;
                    }
                    if switch.active_body && reachable {
                        switch.pending_fallthrough = Some(match switch.pending_fallthrough.take() {
                            Some(old) => join_callback_stacks(&old, &stack)?,
                            None => stack.clone(),
                        });
                    }
                    stack = switch.dispatch_stack.clone();
                    locals = callback_locals_from_stack(&stack);
                    reachable = switch.dispatch_reachable;
                    switch.active_body = false;
                }
                NumericValue::SwitchCaseBody => {
                    let switch = switches.last()?;
                    if switch.phase == SwitchAnalysisPhase::Suppressed {
                        reachable = false;
                        index += 1;
                        continue;
                    }
                    let expression_reachable = reachable;
                    if reachable {
                        stack.pop()?;
                        if stack.len() != switch.dispatch_stack.len() {
                            return None;
                        }
                    }
                    let switch = switches.last_mut()?;
                    if expression_reachable {
                        switch.dispatch_stack = stack.clone();
                    }
                    switch.dispatch_reachable = expression_reachable;
                    if switch.phase == SwitchAnalysisPhase::Discover {
                        reachable = false;
                        switch.active_body = false;
                        index += 1;
                        continue;
                    }
                    let fallthrough = switch.pending_fallthrough.take();
                    let had_fallthrough = fallthrough.is_some();
                    if let Some(fallthrough) = fallthrough {
                        stack = if reachable { join_callback_stacks(&stack, &fallthrough)? } else { fallthrough };
                    }
                    reachable = expression_reachable || had_fallthrough;
                    locals = callback_locals_from_stack(&stack);
                    switch.active_body = true;
                }
                NumericValue::SwitchDefault => {
                    let switch = switches.last_mut()?;
                    if switch.phase == SwitchAnalysisPhase::Suppressed {
                        reachable = false;
                        switch.active_body = false;
                        switch.has_default = true;
                        index += 1;
                        continue;
                    }
                    if switch.active_body && reachable {
                        switch.pending_fallthrough = Some(match switch.pending_fallthrough.take() {
                            Some(old) => join_callback_stacks(&old, &stack)?,
                            None => stack.clone(),
                        });
                    }
                    if switch.phase == SwitchAnalysisPhase::Discover {
                        reachable = false;
                        switch.active_body = false;
                        switch.has_default = true;
                        index += 1;
                        continue;
                    }
                    let default_seed = switch.default_seed.as_ref();
                    stack = default_seed
                        .map(|(stack, _)| stack.clone())
                        .unwrap_or_else(|| switch.dispatch_stack.clone());
                    reachable = default_seed
                        .map(|(_, reachable)| *reachable)
                        .unwrap_or(switch.dispatch_reachable);
                    let fallthrough = switch.pending_fallthrough.take();
                    if let Some(fallthrough) = fallthrough.as_ref() {
                        stack = if reachable { join_callback_stacks(&stack, fallthrough)? } else { fallthrough.clone() };
                        reachable = true;
                    }
                    locals = callback_locals_from_stack(&stack);
                    switch.active_body = true;
                    switch.has_default = true;
                }
                NumericValue::SwitchBreak => {
                    if reachable {
                        let switch = switches.last_mut()?;
                        if stack.len() < switch.base_stack.len() {
                            return None;
                        }
                        let mut exit = stack.clone();
                        exit.truncate(switch.base_stack.len());
                        switch.exits.push(exit);
                        stack.truncate(switch.base_stack.len());
                        reachable = false;
                    }
                }
                NumericValue::SwitchEnd => {
                    let mut switch = switches.pop()?;
                    if switch.phase == SwitchAnalysisPhase::Suppressed {
                        stack = switch.base_stack;
                        locals = switch.base_locals;
                        reachable = false;
                        index += 1;
                        continue;
                    }
                    if switch.phase == SwitchAnalysisPhase::Discover {
                        let seed = (switch.dispatch_stack, switch.dispatch_reachable);
                        let snapshot = *switch.snapshot.take()?;
                        stack = snapshot.stack;
                        locals = snapshot.locals;
                        reachable = snapshot.reachable;
                        branches = snapshot.branches;
                        guards = snapshot.guards;
                        results = snapshot.results;
                        switches = snapshot.switches;
                        loops = snapshot.loops;
                        tries = snapshot.tries;
                        catches = snapshot.catches;
                        early_returns = snapshot.early_returns;
                        switches.push(CallbackSwitchState {
                            start_index: switch.start_index,
                            phase: SwitchAnalysisPhase::Replay,
                            snapshot: None,
                            default_seed: Some(seed),
                            base_stack: switch.base_stack,
                            base_locals: switch.base_locals,
                            dispatch_stack: stack.clone(),
                            dispatch_reachable: reachable,
                            exits: Vec::new(),
                            pending_fallthrough: None,
                            active_body: false,
                            has_default: false,
                        });
                        index = switch.start_index + 1;
                        continue;
                    }
                    if switch.active_body && reachable {
                        if stack.len() != switch.base_stack.len() {
                            return None;
                        }
                        let mut exit = stack.clone();
                        exit.pop()?;
                        switch.exits.push(exit);
                    }
                    if !switch.has_default && switch.dispatch_reachable {
                        let mut unmatched = switch.dispatch_stack;
                        unmatched.pop()?;
                        switch.exits.push(unmatched);
                    }
                    let mut iter = switch.exits.into_iter();
                    if let Some(mut exit) = iter.next() {
                        for other in iter {
                            exit = join_callback_stacks(&exit, &other)?;
                        }
                        stack = exit;
                        locals = callback_locals_from_stack(&stack);
                        reachable = true;
                    } else {
                        stack = switch.base_stack;
                        locals = switch.base_locals;
                        reachable = false;
                    }
                }
                NumericValue::LoopStart => loops.push(CallbackLoopState {
                    start_index: index,
                    base_stack: stack.clone(),
                    header_stack: stack.clone(),
                    base_locals: locals,
                    header_locals: locals,
                    continue_target: None,
                    continue_header: None,
                    exits: Vec::new(),
                    backedges: Vec::new(),
                    continue_edges: Vec::new(),
                }),
                NumericValue::LoopWhile => {
                    let loop_state = loops.last_mut()?;
                    if reachable { stack.pop()?; }
                    if reachable && stack.len() != loop_state.base_stack.len() {
                        return None;
                    }
                    if reachable { loop_state.exits.push(stack.clone()); }
                }
                NumericValue::LoopContinuePoint => {
                    let loop_state = loops.last_mut()?;
                    let continues = std::mem::take(&mut loop_state.continue_edges);
                    let mut iter = continues.into_iter();
                    if let Some(mut incoming) = iter.next() {
                        for next in iter { incoming = join_callback_stacks(&incoming, &next)?; }
                        stack = if reachable { join_callback_stacks(&stack, &incoming)? } else { incoming };
                        locals = callback_locals_from_stack(&stack);
                        reachable = true;
                    }
                    if reachable {
                        loop_state.continue_header = Some(match loop_state.continue_header.take() {
                            Some(previous) => join_callback_stacks(&previous, &stack)?,
                            None => stack.clone(),
                        });
                    }
                    loop_state.continue_target = Some(index);
                }
                NumericValue::LoopBreak(distance) => {
                    let target = loops.len().checked_sub(usize::from(*distance) + 1)?;
                    if reachable {
                        let target_loop = loops.get_mut(target)?;
                        let mut exit = stack.clone();
                        exit.truncate(target_loop.base_stack.len());
                        target_loop.exits.push(exit);
                        stack.truncate(target_loop.base_stack.len());
                        reachable = false;
                    }
                }
                NumericValue::LoopContinue(distance) => {
                    let target = loops.len().checked_sub(usize::from(*distance) + 1)?;
                    if reachable {
                        let target_loop = loops.get_mut(target)?;
                        let mut exit = stack.clone();
                        exit.truncate(target_loop.base_stack.len());
                        target_loop.continue_edges.push(exit);
                        stack.truncate(target_loop.base_stack.len());
                        reachable = false;
                    }
                }
                NumericValue::LoopEnd => {
                    let loop_state = loops.last_mut()?;
                    if reachable && stack.len() != loop_state.base_stack.len() {
                        return None;
                    }
                    if reachable { loop_state.backedges.push(stack.clone()); }
                    if !loop_state.continue_edges.is_empty() {
                        let mut continuation = loop_state.continue_header.clone()?;
                        for incoming in loop_state.continue_edges.drain(..) {
                            continuation = join_callback_stacks(&continuation, &incoming)?;
                        }
                        if loop_state.continue_header.as_ref() != Some(&continuation) {
                            loop_state.continue_header = Some(continuation.clone());
                            let continue_target = loop_state.continue_target?;
                            stack = continuation;
                            locals = callback_locals_from_stack(&stack);
                            reachable = true;
                            index = continue_target + 1;
                            continue;
                        }
                    }
                    let mut next_header = loop_state.header_stack.clone();
                    next_header = join_callback_stacks(&next_header, &loop_state.base_stack)?;
                    for incoming in loop_state.backedges.iter().cloned() {
                        next_header = join_callback_stacks(&next_header, &incoming)?;
                    }
                    if next_header != loop_state.header_stack {
                        loop_state.header_stack = next_header.clone();
                        loop_state.header_locals = callback_locals_from_stack(&next_header);
                        loop_state.backedges.clear();
                        loop_state.continue_edges.clear();
                        let start_index = loop_state.start_index;
                        stack = next_header;
                        locals = callback_locals_from_stack(&stack);
                        reachable = true;
                        index = start_index + 1;
                        continue;
                    }
                    let loop_state = loops.pop()?;
                    let mut exits = loop_state.exits;
                    let has_exit = !exits.is_empty();
                    let mut iter = exits.into_iter();
                    if let Some(mut first) = iter.next() {
                        for exit in iter {
                            first = join_callback_stacks(&first, &exit)?;
                        }
                        stack = first;
                        locals = callback_locals_from_stack(&stack);
                    } else {
                        // No condition-false or break edge reaches this point. This is
                        // an infinite/unreachable continuation; keep the parser valid
                        // and poison local knowledge so it cannot certify a later read.
                        stack = loop_state.base_stack.clone();
                        locals = [CallbackLocalState::Unknown; 8];
                    }
                    reachable = has_exit;
                }
                NumericValue::TryStart | NumericValue::TaggedTryStart => {
                    tries.push(CallbackTryState {
                        base_stack: stack.clone(),
                        tagged: matches!(value, NumericValue::TaggedTryStart),
                        exceptions: Vec::new(),
                    });
                }
                NumericValue::Throw => {
                    if !reachable { index += 1; continue; }
                    if stack.len() <= tries.last()?.base_stack.len() { return None; }
                    let exception = stack.pop()?;
                    let state = tries.last_mut()?;
                    if state.tagged {
                        return None;
                    }
                    let mut thrown = stack[..state.base_stack.len()].to_vec();
                    thrown.push(exception);
                    state.exceptions.push((thrown, locals));
                    reachable = false;
                }
                NumericValue::TaggedThrow(_) => {
                    if !reachable { index += 1; continue; }
                    if stack.len() <= tries.last()?.base_stack.len() { return None; }
                    let exception = stack.pop()?;
                    let state = tries.last_mut()?;
                    if !state.tagged {
                        return None;
                    }
                    let mut thrown = stack[..state.base_stack.len()].to_vec();
                    thrown.push(Some(CallbackValueKind::Number));
                    thrown.push(exception);
                    state.exceptions.push((thrown, locals));
                    reachable = false;
                }
                NumericValue::CheckError => {
                    if !reachable { index += 1; continue; }
                    let state = tries.last_mut()?;
                    if stack.len() < state.base_stack.len() { return None; }
                    let mut thrown = stack[..state.base_stack.len()].to_vec();
                    if state.tagged {
                        thrown.push(Some(CallbackValueKind::Number));
                    }
                    thrown.push(Some(CallbackValueKind::String));
                    state.exceptions.push((thrown, locals));
                }
                NumericValue::UncaughtNumberThrow
                | NumericValue::UncaughtBooleanThrow
                | NumericValue::UncaughtStringThrow
                | NumericValue::UncaughtNumberArrayThrow
                | NumericValue::UncaughtBooleanArrayThrow
                | NumericValue::UncaughtStringArrayThrow
                | NumericValue::UncaughtDictionaryThrow => {
                    if reachable {
                        stack.pop()?;
                        reachable = false;
                    }
                }
                NumericValue::CatchStart => {
                    let state = tries.pop()?;
                    if state.exceptions.is_empty() {
                        // The catch body has no incoming edge. Preserve the normal
                        // try result and skip the unreachable body; interpreting its
                        // instructions as a second live path would poison valid
                        // markerless programs whose try body cannot throw.
                        index = skip_unreachable_catch(&self.0, index)?;
                        continue;
                    }
                    let normal_stack = stack.clone();
                    let normal_locals = locals;
                    let normal_reachable = reachable;
                    let mut exception_iter = state.exceptions.into_iter();
                    let (mut catch_stack, mut catch_locals) = exception_iter.next()?;
                    for (exception_stack, exception_locals) in exception_iter {
                        catch_stack = join_callback_stacks(&catch_stack, &exception_stack)?;
                        catch_locals = join_callback_locals(catch_locals, exception_locals);
                    }
                    catches.push(CallbackCatchState {
                        normal_stack,
                        normal_locals,
                        normal_reachable,
                    });
                    stack = catch_stack;
                    locals = catch_locals;
                    reachable = true;
                }
                NumericValue::TryEnd => {
                    let normal = catches.pop()?;
                    match (normal.normal_reachable, reachable) {
                        (true, true) => {
                            stack = join_callback_stacks(&normal.normal_stack, &stack)?;
                            locals = join_callback_locals(normal.normal_locals, locals);
                        }
                        (true, false) => { stack = normal.normal_stack; locals = normal.normal_locals; reachable = true; }
                        (false, true) => {}
                        (false, false) => { reachable = false; }
                    }
                }
                NumericValue::EarlyReturn => {
                    if !reachable { index += 1; continue; }
                    early_returns.push(Some(stack.pop()?));
                    reachable = false;
                }
                NumericValue::TagMaybePrimitive(kind) if reachable => {
                    let actual = stack.last().copied().flatten()?;
                    let expected = match kind {
                        0 => CallbackValueKind::Number,
                        1 => CallbackValueKind::String,
                        2 => CallbackValueKind::Boolean,
                        3 => CallbackValueKind::Dynamic,
                        _ => return None,
                    };
                    if present_callback_kind(actual) != Some(expected) {
                        return None;
                    }
                    stack.pop()?;
                    stack.push(Some(CallbackValueKind::Dynamic));
                }
                NumericValue::AsBoolean | NumericValue::BooleanNot if reachable => {
                    let actual = stack.last().copied().flatten()?;
                    if !matches!(
                        present_callback_kind(actual),
                        Some(CallbackValueKind::Number | CallbackValueKind::Boolean)
                    ) {
                        return None;
                    }
                    stack.pop()?;
                    stack.push(Some(CallbackValueKind::Boolean));
                }
                _ if !reachable => {}
                _ => {
                    let (consumed, produced) = callback_operation_kind(value)?;
                    validate_callback_operands(value, &stack, consumed)?;
                    stack.truncate(stack.len() - consumed);
                    stack.push(Some(produced));
                }
            }
            if reachable {
                locals = callback_locals_from_stack(&stack);
            }
            index += 1;
        }

        let result = reachable.then(|| stack.last().copied().flatten());
        let mut exits = early_returns;
        if reachable { exits.push(result.flatten()); }
        let mut exits = exits.into_iter();
        let mut result = exits.next()?;
        for exit in exits { result = join_callback_kind(result, exit); }
        match result? {
            CallbackValueKind::Number
            | CallbackValueKind::Boolean
            | CallbackValueKind::String
            | CallbackValueKind::Dynamic
            | CallbackValueKind::MaybeNumber
            | CallbackValueKind::MaybeBoolean
            | CallbackValueKind::MaybeString
            | CallbackValueKind::MaybeDynamic => result,
            CallbackValueKind::ArrayReference
            | CallbackValueKind::DictionaryReference
            | CallbackValueKind::ObjectReference
            | CallbackValueKind::TupleReference
            | CallbackValueKind::NativeHandle
            | CallbackValueKind::MaybeArrayReference
            | CallbackValueKind::Unknown => None,
        }
    }

    /// Returns the callback ABI representation proven by the parsed program.
    /// Maybe values use their present-family representation here; their
    /// undefined/null state is carried separately through CALL_PRESENT.
    fn callback_result_kind_code(&self) -> Option<u8> {
        callback_result_kind_code(self.callback_result_kind()?)
    }

    fn callback_result_kind_code_for_environment(
        &self,
        argument_kinds: &[u8],
        dynamic_pair_starts: &[u8],
    ) -> Option<u8> {
        callback_result_kind_code(self.callback_result_kind_for_environment(
            argument_kinds,
            dynamic_pair_starts,
            true,
        )?)
    }
}

fn callback_result_kind_code(kind: CallbackValueKind) -> Option<u8> {
    use CallbackValueKind as K;
    Some(match kind {
            K::Number | K::MaybeNumber => 0,
            K::Boolean | K::MaybeBoolean => 1,
            K::String | K::MaybeString => 2,
            K::Dynamic | K::MaybeDynamic => 3,
            _ => return None,
        })
}

fn callback_value_kind_from_code(code: u8) -> Option<CallbackValueKind> {
    use CallbackValueKind as K;
    Some(match code {
        0 => K::Number,
        1 => K::Boolean,
        2 => K::String,
        3 => K::Dynamic,
        4 => K::ArrayReference,
        5 => K::DictionaryReference,
        6 => K::ObjectReference,
        7 => K::TupleReference,
        8 => K::NativeHandle,
        9 => K::Unknown,
        10 => K::MaybeNumber,
        11 => K::MaybeBoolean,
        12 => K::MaybeString,
        13 => K::MaybeDynamic,
        14 => K::MaybeArrayReference,
        _ => return None,
    })
}

fn skip_unreachable_catch(program: &[NumericValue], catch_start: usize) -> Option<usize> {
    let mut nested_tries = 0usize;
    for (index, value) in program.iter().enumerate().skip(catch_start + 1) {
        match value {
            NumericValue::TryStart | NumericValue::TaggedTryStart => nested_tries += 1,
            NumericValue::TryEnd if nested_tries > 0 => nested_tries -= 1,
            NumericValue::TryEnd => return Some(index + 1),
            _ => {}
        }
    }
    None
}

fn is_callback_control(value: &NumericValue) -> bool {
    matches!(
        value,
        NumericValue::ConditionalStart
            | NumericValue::PresentConditionalStart
            | NumericValue::ConditionalAlternate
            | NumericValue::ShortCircuit(_)
            | NumericValue::ShortCircuitEnd
            | NumericValue::GuardStart
            | NumericValue::GuardAlternate
            | NumericValue::GuardEnd
            | NumericValue::ResultStart
            | NumericValue::ResultReturn(_)
            | NumericValue::ResultEnd
            | NumericValue::SwitchStart
            | NumericValue::SwitchCaseStart
            | NumericValue::SwitchCaseBody
            | NumericValue::SwitchDefault
            | NumericValue::SwitchBreak
            | NumericValue::SwitchEnd
            | NumericValue::LoopStart
            | NumericValue::LoopWhile
            | NumericValue::LoopContinuePoint
            | NumericValue::LoopBreak(_)
            | NumericValue::LoopContinue(_)
            | NumericValue::LoopEnd
            | NumericValue::TryStart
            | NumericValue::TaggedTryStart
            | NumericValue::Throw
            | NumericValue::TaggedThrow(_)
            | NumericValue::CheckError
            | NumericValue::UncaughtNumberThrow
            | NumericValue::UncaughtBooleanThrow
            | NumericValue::UncaughtStringThrow
            | NumericValue::UncaughtNumberArrayThrow
            | NumericValue::UncaughtBooleanArrayThrow
            | NumericValue::UncaughtStringArrayThrow
            | NumericValue::UncaughtDictionaryThrow
            | NumericValue::CatchStart
            | NumericValue::TryEnd
            | NumericValue::EarlyReturn
    )
}

#[derive(Clone)]
struct CallbackBranchState {
    base_stack: Vec<Option<CallbackValueKind>>,
    base_locals: [CallbackLocalState; 8],
    base_reachable: bool,
    first_stack: Option<Vec<Option<CallbackValueKind>>>,
    first_locals: Option<[CallbackLocalState; 8]>,
    first_reachable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallbackLocalState {
    Uninitialized,
    Known(CallbackValueKind),
    Unknown,
}

#[derive(Clone)]
struct CallbackResultState {
    base_stack: Vec<Option<CallbackValueKind>>,
    returns: Vec<Vec<Option<CallbackValueKind>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SwitchAnalysisPhase {
    Discover,
    Replay,
    Suppressed,
}

#[derive(Clone)]
struct CallbackSwitchState {
    start_index: usize,
    phase: SwitchAnalysisPhase,
    snapshot: Option<Box<CallbackAnalyzerSnapshot>>,
    default_seed: Option<(Vec<Option<CallbackValueKind>>, bool)>,
    base_stack: Vec<Option<CallbackValueKind>>,
    base_locals: [CallbackLocalState; 8],
    dispatch_stack: Vec<Option<CallbackValueKind>>,
    dispatch_reachable: bool,
    exits: Vec<Vec<Option<CallbackValueKind>>>,
    pending_fallthrough: Option<Vec<Option<CallbackValueKind>>>,
    active_body: bool,
    has_default: bool,
}

#[derive(Clone)]
struct CallbackLoopState {
    start_index: usize,
    base_stack: Vec<Option<CallbackValueKind>>,
    header_stack: Vec<Option<CallbackValueKind>>,
    base_locals: [CallbackLocalState; 8],
    header_locals: [CallbackLocalState; 8],
    continue_target: Option<usize>,
    continue_header: Option<Vec<Option<CallbackValueKind>>>,
    exits: Vec<Vec<Option<CallbackValueKind>>>,
    backedges: Vec<Vec<Option<CallbackValueKind>>>,
    continue_edges: Vec<Vec<Option<CallbackValueKind>>>,
}

#[derive(Clone)]
struct CallbackTryState {
    base_stack: Vec<Option<CallbackValueKind>>,
    tagged: bool,
    exceptions: Vec<(Vec<Option<CallbackValueKind>>, [CallbackLocalState; 8])>,
}

#[derive(Clone)]
struct CallbackCatchState {
    normal_stack: Vec<Option<CallbackValueKind>>,
    normal_locals: [CallbackLocalState; 8],
    normal_reachable: bool,
}

#[derive(Clone)]
struct CallbackAnalyzerSnapshot {
    stack: Vec<Option<CallbackValueKind>>,
    locals: [CallbackLocalState; 8],
    reachable: bool,
    branches: Vec<CallbackBranchState>,
    guards: Vec<CallbackBranchState>,
    results: Vec<CallbackResultState>,
    switches: Vec<CallbackSwitchState>,
    loops: Vec<CallbackLoopState>,
    tries: Vec<CallbackTryState>,
    catches: Vec<CallbackCatchState>,
    early_returns: Vec<Option<CallbackValueKind>>,
}

impl CallbackSourceKind {
    fn representation(self) -> Option<CallbackValueKind> {
        match self {
            Self::Argument(kind) | Self::Local(kind) | Self::Field(kind) | Self::Constant(kind) => {
                Some(kind)
            }
            Self::MaybeField(kind) => Some(maybe_callback_kind(kind)?),
        }
    }
}

fn maybe_callback_kind(kind: CallbackValueKind) -> Option<CallbackValueKind> {
    use CallbackValueKind as K;
    Some(match kind {
        K::Number => K::MaybeNumber,
        K::Boolean => K::MaybeBoolean,
        K::String => K::MaybeString,
        K::Dynamic => K::MaybeDynamic,
        K::ArrayReference => K::MaybeArrayReference,
        _ => return None,
    })
}

fn present_callback_kind(kind: CallbackValueKind) -> Option<CallbackValueKind> {
    use CallbackValueKind as K;
    Some(match kind {
        K::MaybeNumber => K::Number,
        K::MaybeBoolean => K::Boolean,
        K::MaybeString => K::String,
        K::MaybeDynamic => K::Dynamic,
        K::MaybeArrayReference => K::ArrayReference,
        kind => kind,
    })
}

fn join_callback_kind(
    left: Option<CallbackValueKind>,
    right: Option<CallbackValueKind>,
) -> Option<CallbackValueKind> {
    match (left, right) {
        (Some(left), Some(right)) if left == right && left != CallbackValueKind::Unknown => Some(left),
        (Some(left), Some(right)) => join_callback_maybe_kind(left, right),
        _ => None,
    }
}

fn join_callback_maybe_kind(left: CallbackValueKind, right: CallbackValueKind) -> Option<CallbackValueKind> {
    use CallbackValueKind as K;
    Some(match (left, right) {
        (K::Number, K::MaybeNumber) | (K::MaybeNumber, K::Number) | (K::MaybeNumber, K::MaybeNumber) => K::MaybeNumber,
        (K::Boolean, K::MaybeBoolean) | (K::MaybeBoolean, K::Boolean) | (K::MaybeBoolean, K::MaybeBoolean) => K::MaybeBoolean,
        (K::String, K::MaybeString) | (K::MaybeString, K::String) | (K::MaybeString, K::MaybeString) => K::MaybeString,
        (K::Dynamic, K::MaybeDynamic) | (K::MaybeDynamic, K::Dynamic) | (K::MaybeDynamic, K::MaybeDynamic) => K::MaybeDynamic,
        (K::ArrayReference, K::MaybeArrayReference) | (K::MaybeArrayReference, K::ArrayReference) | (K::MaybeArrayReference, K::MaybeArrayReference) => K::MaybeArrayReference,
        _ => return None,
    })
}

fn callback_locals_from_stack(stack: &[Option<CallbackValueKind>]) -> [CallbackLocalState; 8] {
    std::array::from_fn(|index| match stack.get(index).copied().flatten() {
        Some(kind) => CallbackLocalState::Known(kind),
        None if index < stack.len() => CallbackLocalState::Unknown,
        None => CallbackLocalState::Uninitialized,
    })
}

fn join_callback_stacks(
    left: &[Option<CallbackValueKind>],
    right: &[Option<CallbackValueKind>],
) -> Option<Vec<Option<CallbackValueKind>>> {
    (left.len() == right.len()).then(|| {
        left.iter()
            .zip(right)
            .map(|(left, right)| join_callback_kind(*left, *right))
            .collect()
    })
}

fn join_callback_locals(
    left: [CallbackLocalState; 8],
    right: [CallbackLocalState; 8],
) -> [CallbackLocalState; 8] {
    std::array::from_fn(|index| match (left[index], right[index]) {
        (CallbackLocalState::Uninitialized, CallbackLocalState::Uninitialized) => {
            CallbackLocalState::Uninitialized
        }
        (CallbackLocalState::Known(left), CallbackLocalState::Known(right)) if left == right => {
            CallbackLocalState::Known(left)
        }
        _ => CallbackLocalState::Unknown,
    })
}

fn join_callback_exits(
    exits: &[Vec<Option<CallbackValueKind>>],
) -> Option<Vec<Option<CallbackValueKind>>> {
    let mut iter = exits.iter();
    let mut joined = iter.next()?.clone();
    for exit in iter {
        joined = join_callback_stacks(&joined, exit)?;
    }
    Some(joined)
}

fn callback_operation_kind(value: &NumericValue) -> Option<(usize, CallbackValueKind)> {
    use CallbackValueKind::{ArrayReference, Boolean, DictionaryReference, Dynamic, Number, String};
    use NumericValue as V;
    let output = match value {
        V::Operation(_) | V::Bitwise(_) | V::BitNot | V::Absolute | V::Negate | V::Maximum
        | V::Minimum | V::Atan2 | V::Hypot | V::Imul | V::ParseFloat | V::ParseInt
        | V::StringCharCodeAt | V::StringLength | V::ArrayLength
        | V::DynamicArrayLength | V::NumberArrayIndexOf | V::BoolArrayIndexOf
        | V::StringArrayIndexOf | V::DynamicArrayIndexOf | V::StringIndexOf | V::StringIndexOfAt
        | V::StringLastIndexOf | V::StringLastIndexOfAt | V::NumberArrayLastIndexOf
        | V::BoolArrayLastIndexOf | V::StringArrayLastIndexOf | V::DynamicArrayLastIndexOf
        | V::StringCompare
        | V::NumberArrayMin | V::NumberArrayMax | V::NumberArrayHypot | V::NumberArrayReduce(_, _, _)
        | V::NumberDictionaryGet | V::DictionaryLength | V::Power | V::ReversePower | V::Remainder
        | V::ReverseRemainder | V::UnaryMath(_) | V::StringToNumber | V::DynamicToNumber
        | V::UntagNumber | V::MathRandom | V::DateNow | V::PerformanceNow | V::ProcessPid
        | V::ProcessPpid | V::NumberArrayPush | V::NumberArrayPostSet | V::NumberDictionaryPostSet
        | V::StringArrayPush | V::BoolArrayPush | V::DynamicArrayPush | V::NumberArrayUnshift
        | V::StringArrayUnshift | V::BoolArrayUnshift | V::DynamicArrayUnshift => Number,
        V::Compare(_) | V::StrictPresenceMismatch | V::IsFinite | V::IsInteger | V::IsNaN | V::IsSafeInteger
        | V::NumberSameValue | V::StringSameValue | V::ReferenceSameValue | V::DynamicToBoolean
        | V::AsBoolean | V::BooleanNot
        | V::DynamicCompare(_) | V::StringEndsWith | V::StringEndsWithAt | V::StringIncludes
        | V::StringIncludesAt | V::StringIsWellFormed | V::IsArray | V::IsNotArray
        | V::DynamicIsArray | V::StringTruthy | V::StringStartsWith | V::StringStartsWithAt
        | V::NumberArrayQuantifier(_, _) | V::NumberArrayIncludes | V::BoolArrayIncludes
        | V::StringArrayIncludes | V::DynamicArrayIncludes | V::SetIsSubsetOf
        | V::SetIsSupersetOf | V::SetIsDisjointFrom | V::DictionaryHasOwn | V::DictionaryIn
        | V::DictionaryDelete | V::DictionaryStrictDelete | V::UntagBoolean
        => Boolean,
        V::PrimitiveArrayTruthy(_, mode) if *mode < 2 => Boolean,
        V::PrimitiveArrayTruthy(kind, mode) if *mode == 2 || *mode == 4 => {
            maybe_callback_kind(match kind {
                0 => Number,
                1 => Boolean,
                2 => String,
                _ => return None,
            })?
        }
        V::PrimitiveArrayTruthy(_, mode) if *mode == 3 || *mode == 5 => Number,
        V::PrimitiveArrayTruthy(_, 6) => ArrayReference,
        V::DynamicArrayTruthy(mode) if *mode < 2 => Boolean,
        V::DynamicArrayTruthy(mode) if *mode == 2 || *mode == 4 => {
            CallbackValueKind::MaybeDynamic
        }
        V::DynamicArrayTruthy(6) => Dynamic,
        V::DynamicArrayTruthy(mode) if *mode == 3 || *mode == 5 => Number,
        V::NumberArraySet => Number,
        V::BoolArraySet => Boolean,
        V::StringArraySet => String,
        V::DynamicArrayCompare(_, mode) if *mode < 2 => Boolean,
        V::DynamicArrayCompare(_, mode) if *mode == 2 || *mode == 4 => {
            CallbackValueKind::MaybeDynamic
        }
        V::DynamicArrayCompare(_, 6) => Dynamic,
        V::DynamicArrayCompare(_, mode) if *mode == 3 || *mode == 5 => Number,
        V::PrimitiveArrayCompare(kind, _, mode) if *mode < 2 => Boolean,
        V::PrimitiveArrayCompare(kind, _, mode) if *mode == 2 || *mode == 4 => {
            maybe_callback_kind(match kind {
                0 => Number,
                1 => Boolean,
                2 => String,
                _ => return None,
            })?
        }
        V::PrimitiveArrayCompare(_, _, mode) if *mode == 3 || *mode == 5 => Number,
        V::PrimitiveArrayCompare(_, _, 6) => ArrayReference,
        V::PrimitiveArrayJitScan(_, mode, _) if *mode < 2 => Boolean,
        V::PrimitiveArrayJitScan(kind, mode, _) if *mode == 2 || *mode == 4 => {
            maybe_callback_kind(match kind {
                0 => Number,
                1 => Boolean,
                2 => String,
                _ => return None,
            })?
        }
        V::PrimitiveArrayJitScan(_, mode, _) if *mode == 3 || *mode == 5 => Number,
        V::PrimitiveArrayJitScan(_, 6, _) => ArrayReference,
        V::DynamicArrayJitScan(_, mode) if *mode < 2 => Boolean,
        V::DynamicArrayJitScan(_, mode) if *mode == 2 || *mode == 4 => {
            CallbackValueKind::MaybeDynamic
        }
        V::DynamicArrayJitScan(_, 6) => Dynamic,
        V::DynamicArrayJitScan(_, mode) if *mode == 3 || *mode == 5 => Number,
        V::TypeOfNumber | V::TypeOfBoolean | V::TypeOfString | V::TypeOfObject
        | V::TypeOfDynamic | V::StringCharAt | V::StringConcat
        | V::NumberToString | V::BooleanToString | V::DynamicToString
        | V::NumberToPrecision | V::NumberToRadixString | V::NumberToExponentialShortest
        | V::NumberToExponential | V::NumberToFixed | V::StringFromCharCode
        | V::StringFromCodePoint | V::StringSlice | V::StringSliceRange | V::StringSubstring
        | V::StringSubstringRange | V::StringToLowerCase | V::StringToUpperCase
        | V::StringTrim | V::StringTrimEnd | V::StringTrimStart | V::StringRepeat
        | V::StringNormalize | V::DictionaryKeyAt | V::StringReplace
        | V::StringReplaceAll | V::StringPadEnd | V::StringPadStart | V::StringToWellFormed
        | V::UntagString | V::NumberArrayJoin
        | V::BoolArrayJoin | V::StringArrayJoin | V::DynamicArrayJoin => String,
        V::DynamicAdd | V::TagNumber | V::TagString | V::TagBoolean | V::TagMaybePrimitive(_) | V::TagAggregate(_)
        | V::DynamicTag | V::DynamicArrayAppend => Dynamic,
        V::DynamicArraySlice | V::DynamicArrayConcat | V::DynamicArrayToReversed
        | V::DynamicArrayReverse | V::DynamicArrayToSorted | V::DynamicArraySort
        | V::DynamicArrayMapIdentity | V::DynamicArrayFill | V::DynamicArrayCopyWithin
        | V::DynamicArrayWith | V::DynamicArraySet | V::DynamicArraySplice
        | V::DynamicArrayToSpliced => Dynamic,
        V::DynamicArrayConvert(_) | V::PrimitiveArrayConvert(_, _) | V::PrimitiveArrayMap(_, _)
        | V::NumberArrayFilter(_) | V::PrimitiveArrayJitMap(_, _, _)
        | V::DynamicArrayJitMap(_, _) => ArrayReference,
        V::ArrayValue | V::EmptyArray | V::ArraySlice | V::ArrayConcat | V::ArrayToReversed
        | V::ArrayReverse | V::StringToArray | V::StringSplit
        | V::NumberArrayToSorted | V::NumberArrayToSortedBy(_) | V::StringArrayToSorted
        | V::StringArrayToSortedDescending | V::BoolArrayToSorted | V::DynamicArrayToSorted
        | V::NumberArraySortBy(_) | V::NumberArraySort | V::StringArraySort
        | V::StringArraySortDescending | V::BoolArraySort | V::DynamicArraySort
        | V::NumberArrayMap(_, _) | V::NumberArrayIndexMap(_, _) | V::NumberArraySelectMap(_, _)
        | V::NumberArrayUnaryMap(_) | V::NumberArrayMathMap(_)
        | V::NumberArrayAppend | V::StringArrayAppend | V::BoolArrayAppend
        | V::NumberArrayFill | V::StringArrayFill | V::BoolArrayFill | V::ArrayCopyWithin
        | V::ArraySplice | V::ArrayToSpliced
        | V::NumberArrayWith | V::StringArrayWith | V::BoolArrayWith | V::DictionaryKeys
        | V::NumberDictionaryValues | V::BoolDictionaryValues | V::StringDictionaryValues
        | V::NumberDictionaryEntries | V::BoolDictionaryEntries | V::StringDictionaryEntries => {
            ArrayReference
        }
        V::FixedObjectNew(_) => CallbackValueKind::ObjectReference,
        V::FixedTupleNew(_) | V::FixedWideTupleNew(_) => CallbackValueKind::TupleReference,
        V::EmptyDictionary | V::DictionaryAppend(_) | V::DictionaryStaticAppend(_, _)
        | V::DictionaryAssign | V::DictionaryFromNumberEntries | V::DictionaryFromBoolEntries
        | V::DictionaryFromStringEntries | V::SetLikeDictionary => {
            DictionaryReference
        }
        V::SetUnion | V::SetIntersection | V::SetDifference | V::SetSymmetricDifference
        | V::SetFromArray => CallbackValueKind::ObjectReference,
        V::NumberArrayPop | V::NumberArrayShift => CallbackValueKind::MaybeNumber,
        V::BoolArrayPop | V::BoolArrayShift => CallbackValueKind::MaybeBoolean,
        V::StringArrayPop | V::StringArrayShift => CallbackValueKind::MaybeString,
        V::DynamicArrayPop | V::DynamicArrayShift => CallbackValueKind::MaybeDynamic,
        V::NumberArrayFind(_, mode) if *mode % 2 == 0 => CallbackValueKind::MaybeNumber,
        V::NumberArrayFind(_, _) => Number,
        V::NumberArrayAt | V::NumberArrayGet => CallbackValueKind::MaybeNumber,
        V::StringArrayAt | V::StringArrayGet => CallbackValueKind::MaybeString,
        V::BoolArrayAt | V::BoolArrayGet => CallbackValueKind::MaybeBoolean,
        V::DynamicArrayAt => CallbackValueKind::MaybeDynamic,
        V::StringAt => CallbackValueKind::MaybeString,
        V::StringCodePointAt => CallbackValueKind::MaybeNumber,
        V::StringDictionaryGet => String,
        V::BoolDictionaryGet => Boolean,
        V::OptionalNumberDictionaryGet => CallbackValueKind::MaybeNumber,
        V::OptionalBoolDictionaryGet => CallbackValueKind::MaybeBoolean,
        V::OptionalStringDictionaryGet => CallbackValueKind::MaybeString,
        _ => return None,
    };
    let consumed = match value {
        V::Operation(_) | V::Compare(_) | V::StrictPresenceMismatch | V::DynamicCompare(_) | V::DynamicAdd | V::StringConcat
        | V::StringCompare | V::NumberSameValue | V::StringSameValue | V::ReferenceSameValue
        | V::Maximum | V::Minimum | V::Atan2 | V::Hypot | V::Power | V::ReversePower
        | V::Remainder | V::ReverseRemainder | V::StringAt | V::StringCharAt
        | V::StringCharCodeAt | V::StringCodePointAt | V::ParseInt | V::NumberToFixed
        | V::NumberToPrecision | V::NumberToRadixString | V::NumberToExponential
        | V::StringStartsWith | V::StringEndsWith | V::StringIncludes | V::StringIndexOf
        | V::StringLastIndexOf | V::StringRepeat | V::StringNormalize | V::StringSlice
        | V::StringSubstring | V::ArrayConcat | V::DynamicArrayConcat | V::NumberArrayFind(_, _)
        | V::NumberArrayAt
        | V::BoolArrayAt | V::StringArrayAt | V::DynamicArrayAt | V::NumberArrayGet
        | V::BoolArrayGet | V::StringArrayGet | V::NumberDictionaryGet | V::BoolDictionaryGet
        | V::StringDictionaryGet | V::OptionalNumberDictionaryGet | V::OptionalBoolDictionaryGet
        | V::OptionalStringDictionaryGet | V::NumberArrayJoin | V::BoolArrayJoin | V::StringArrayJoin
        | V::DynamicArrayJoin | V::NumberArrayPush | V::StringArrayPush | V::BoolArrayPush
        | V::DynamicArrayPush | V::NumberArrayUnshift | V::StringArrayUnshift
        | V::BoolArrayUnshift | V::DynamicArrayUnshift | V::NumberArrayAppend
        | V::StringArrayAppend | V::BoolArrayAppend | V::DynamicArrayAppend => 2,
        V::StringSliceRange | V::StringSubstringRange | V::StringPadStart | V::StringPadEnd
        | V::StringStartsWithAt | V::StringEndsWithAt | V::StringIncludesAt
        | V::StringIndexOfAt | V::StringLastIndexOfAt | V::StringReplace | V::StringReplaceAll
        | V::StringSplit | V::ArraySlice | V::DynamicArraySlice | V::NumberArrayIncludes
        | V::BoolArrayIncludes | V::StringArrayIncludes | V::DynamicArrayIncludes
        | V::NumberArrayIndexOf | V::BoolArrayIndexOf | V::StringArrayIndexOf
        | V::DynamicArrayIndexOf | V::NumberArrayLastIndexOf | V::BoolArrayLastIndexOf
        | V::StringArrayLastIndexOf | V::DynamicArrayLastIndexOf | V::NumberArrayWith
        | V::StringArrayWith | V::BoolArrayWith | V::DynamicArrayWith | V::DynamicArraySet
        | V::NumberArraySet | V::StringArraySet | V::BoolArraySet => 3,
        V::PrimitiveArrayCompare(_, _, _) | V::DynamicArrayCompare(_, _) => 2,
        V::NumberArrayReduce(_, _, _) => 2,
        V::NumberArrayMap(_, _) | V::NumberArraySelectMap(_, _) | V::NumberArrayBranchMap(_) => 2,
        V::NumberArrayJitReduce(_, _, false) => 3,
        V::NumberArrayJitReduce(_, _, true) => 4,
        V::PrimitiveArrayJitMap(_, _, false) | V::DynamicArrayJitMap(_, false)
        | V::PrimitiveArrayJitScan(_, _, false) | V::DynamicArrayJitScan(_, false) => 2,
        V::PrimitiveArrayJitMap(_, _, true) | V::DynamicArrayJitMap(_, true)
        | V::PrimitiveArrayJitScan(_, _, true) | V::DynamicArrayJitScan(_, true) => 3,
        V::NumberArrayFill | V::StringArrayFill | V::BoolArrayFill | V::DynamicArrayFill
        | V::ArrayCopyWithin | V::DynamicArrayCopyWithin | V::ArraySplice | V::ArrayToSpliced
        | V::DynamicArraySplice | V::DynamicArrayToSpliced | V::NumberArrayFill
        | V::StringArrayFill | V::BoolArrayFill | V::ArrayCopyWithin => 4,
        V::NumberArrayPostSet | V::NumberDictionaryPostSet => 3,
        _ => 1,
    };
    Some((consumed, output))
}

fn validate_callback_operands(
    value: &NumericValue,
    stack: &[Option<CallbackValueKind>],
    consumed: usize,
) -> Option<()> {
    use CallbackValueKind as K;
    use NumericValue as V;
    if consumed > stack.len() {
        return None;
    }
    let operands = &stack[stack.len() - consumed..];
    let require = |index: usize, expected: K| {
        let actual = operands.get(index).copied().flatten()?;
        (actual == expected).then_some(())
    };
    let require_any = |index: usize, expected: &[K]| {
        let actual = operands.get(index).copied().flatten()?;
        expected.contains(&actual).then_some(())
    };
    match value {
        V::StringToNumber => require_any(0, &[K::String, K::MaybeString])?,
        V::StringLength | V::StringToArray | V::StringToWellFormed
        | V::StringToLowerCase | V::StringToUpperCase | V::StringTrim | V::StringTrimEnd
        | V::StringTrimStart | V::StringNormalize | V::StringIsWellFormed | V::StringTruthy => {
            require(0, K::String)?;
        }
        V::StringCompare | V::StringConcat | V::StringSameValue => {
            require(0, K::String)?;
            require(1, K::String)?;
        }
        V::StringAt | V::StringCharAt | V::StringCodePointAt => {
            require(0, K::String)?;
            require(1, K::Number)?;
        }
        V::DynamicToString | V::DynamicToNumber | V::DynamicToBoolean => {
            require_any(0, &[K::Dynamic, K::MaybeDynamic])?;
        }
        V::DynamicTag
        | V::UntagNumber | V::UntagString | V::UntagBoolean | V::UntagNumberArray
        | V::UntagBooleanArray | V::UntagStringArray | V::UntagArray
        | V::UntagNumberDictionary | V::UntagBooleanDictionary | V::UntagStringDictionary
        | V::UntagDictionary | V::UntagObject | V::UntagTuple | V::DynamicIsArray => {
            require(0, K::Dynamic)?;
        }
        V::DynamicCompare(_) | V::DynamicAdd => {
            require(0, K::Dynamic)?;
            require(1, K::Dynamic)?;
        }
        V::NumberToString => require_any(0, &[K::Number, K::MaybeNumber])?,
        V::NumberToFixed | V::NumberToPrecision
        | V::NumberToRadixString | V::NumberToExponential | V::NumberToExponentialShortest
        | V::Absolute | V::Negate | V::UnaryMath(_) | V::IsFinite | V::IsInteger | V::IsNaN
        | V::IsSafeInteger | V::AsBoolean | V::BooleanNot => {
            require(0, K::Number)?;
        }
        V::BooleanToString => require_any(0, &[K::Boolean, K::MaybeBoolean])?,
        V::NumberSameValue => {
            require(0, K::Number)?;
            require(1, K::Number)?;
        }
        V::StringIndexOf | V::StringLastIndexOf | V::StringIncludes | V::StringStartsWith
        | V::StringEndsWith => {
            require(0, K::String)?;
            require(1, K::String)?;
        }
        V::StringCharCodeAt => {
            require(0, K::String)?;
            require(1, K::Number)?;
        }
        V::ArrayLength => {
            let actual = operands.first().copied().flatten()?;
            if !matches!(actual, K::ArrayReference | K::MaybeArrayReference) {
                return None;
            }
        }
        V::DynamicArrayLength => require(0, K::Dynamic)?,
        V::NumberArrayAt | V::NumberArrayGet | V::BoolArrayAt | V::BoolArrayGet
        | V::StringArrayAt | V::StringArrayGet => {
            let actual = operands.first().copied().flatten()?;
            if !matches!(actual, K::ArrayReference | K::MaybeArrayReference) {
                return None;
            }
            require(1, K::Number)?;
        }
        V::DynamicArrayAt => {
            require(0, K::Dynamic)?;
            require(1, K::Number)?;
        }
        V::ReferenceSameValue => {
            let reference = |kind| matches!(kind, K::ArrayReference | K::DictionaryReference | K::ObjectReference | K::TupleReference | K::NativeHandle);
            if !reference(operands.first().copied().flatten()?)
                || !reference(operands.get(1).copied().flatten()?)
            {
                return None;
            }
        }
        _ => {}
    }
    Some(())
}
