// Direct-HIR regression controls for discarding a Promise-bearing tagged union.
// These controls are authored for a later user-run verification lane; they
// were not compiled or executed as part of this source-only draft.

fn union_discard_three_members() -> Vec<HirType> {
    vec![
        HirType::Promise(Box::new(HirType::F64)),
        HirType::Str,
        HirType::Promise(Box::new(HirType::I64)),
    ]
}

fn union_discard_owned_promise(resolved: HirType) -> HirExpr {
    let resolve = HirType::Function(
        if resolved == HirType::Void { vec![] } else { vec![resolved.clone()] },
        Box::new(HirType::Void),
    );
    let reject = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));
    let executor = HirExpr::Lambda(
        Vec::new(),
        vec![
            HirParam { name: "resolve_discard_probe".into(), ty: resolve },
            HirParam { name: "reject_discard_probe".into(), ty: reject },
        ],
        HirType::Void,
        Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("resolve_discard_probe".into())),
            if resolved == HirType::Void {
                vec![]
            } else if resolved == HirType::I64 {
                vec![HirExpr::Lit(HirLit::I64(17))]
            } else {
                vec![HirExpr::Lit(HirLit::F64(17.0))]
            },
        )),
    );
    HirExpr::PromiseNew(Box::new(executor), resolved, false, false)
}

fn union_discard_call(name: &str) -> HirExpr {
    HirExpr::Call(Box::new(HirExpr::Var(name.into())), Vec::new())
}

fn union_discard_function(
    name: &str,
    params: Vec<HirParam>,
    ret: HirType,
    body: Vec<HirStmt>,
) -> HirFunction {
    HirFunction { name: name.into(), params, ret, is_async: false, body }
}

fn union_discard_expression_probe(name: &str, params: Vec<HirParam>, expression: HirExpr) -> HirFunction {
    union_discard_function(name, params, HirType::Void, vec![
        HirStmt::Expr(expression),
        HirStmt::Return(None),
    ])
}

fn union_discard_promise_producer(name: &str, resolved: HirType) -> HirFunction {
    union_discard_function(name, vec![], HirType::Promise(Box::new(resolved.clone())), vec![
        HirStmt::Return(Some(union_discard_owned_promise(resolved))),
    ])
}

fn union_discard_union_producer(
    name: &str,
    members: &[HirType],
    value: HirExpr,
) -> HirFunction {
    union_discard_function(name, vec![], HirType::Union(members.to_vec()), vec![
        HirStmt::Return(Some(value)),
    ])
}

#[derive(Clone, Copy)]
enum UnionDiscardValueFamily {
    Optional,
    Nullable,
    Nullish,
}

fn union_discard_container_type(family: UnionDiscardValueFamily, payload: &HirType) -> HirType {
    match family {
        UnionDiscardValueFamily::Optional => HirType::Optional(Box::new(payload.clone())),
        UnionDiscardValueFamily::Nullable => HirType::Nullable(Box::new(payload.clone())),
        UnionDiscardValueFamily::Nullish => HirType::Nullish(Box::new(payload.clone())),
    }
}

fn union_discard_some_value(
    family: UnionDiscardValueFamily,
    value: HirExpr,
    payload: &HirType,
) -> HirExpr {
    let some = match family {
        UnionDiscardValueFamily::Optional => HirExpr::OptionalSome(Box::new(value), payload.clone()),
        UnionDiscardValueFamily::Nullable => HirExpr::NullableSome(Box::new(value), payload.clone()),
        UnionDiscardValueFamily::Nullish => HirExpr::NullishSome(Box::new(value), payload.clone()),
    };
    let typed = HirExpr::TypedClosure(
        union_discard_container_type(family, payload),
        Box::new(some),
    );
    match family {
        UnionDiscardValueFamily::Optional => HirExpr::OptionalValue(Box::new(typed), payload.clone()),
        UnionDiscardValueFamily::Nullable => HirExpr::NullableValue(Box::new(typed), payload.clone()),
        UnionDiscardValueFamily::Nullish => HirExpr::NullishValue(Box::new(typed), payload.clone()),
    }
}

fn union_discard_roundtrip_some_value(
    family: UnionDiscardValueFamily,
    value: HirExpr,
    payload: &HirType,
) -> HirExpr {
    let container = union_discard_container_type(family, payload);
    let some = match family {
        UnionDiscardValueFamily::Optional => HirExpr::OptionalSome(Box::new(value), payload.clone()),
        UnionDiscardValueFamily::Nullable => HirExpr::NullableSome(Box::new(value), payload.clone()),
        UnionDiscardValueFamily::Nullish => HirExpr::NullishSome(Box::new(value), payload.clone()),
    };
    let typed = HirExpr::TypedClosure(container.clone(), Box::new(some));
    let members = vec![container, HirType::F64];
    let injected = HirExpr::UnionInject(Box::new(typed), 0, members.clone());
    let selected = HirExpr::UnionValue(Box::new(injected), 0, members);
    match family {
        UnionDiscardValueFamily::Optional => HirExpr::OptionalValue(Box::new(selected), payload.clone()),
        UnionDiscardValueFamily::Nullable => HirExpr::NullableValue(Box::new(selected), payload.clone()),
        UnionDiscardValueFamily::Nullish => HirExpr::NullishValue(Box::new(selected), payload.clone()),
    }
}

fn compile_union_discard_program(program: &HirProgram, module_name: &str) -> String {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, module_name);
    compiler.compile_program(program).unwrap();
    compiler.module.verify().unwrap();
    compiler.print_to_string()
}

fn union_discard_function_body(ir: &str, function: &str) -> String {
    let signature = format!("@{function}(");
    let body = ir
        .lines()
        .skip_while(|line| !(line.starts_with("define ") && line.contains(&signature)))
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>();
    assert!(!body.is_empty(), "missing function {function} in IR:\n{ir}");
    body.join("\n")
}

fn union_discard_all_function_bodies(ir: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    let mut in_function = false;
    for line in ir.lines() {
        if !in_function && line.starts_with("define ") {
            in_function = true;
        }
        if in_function {
            current.push(line);
            if line == "}" {
                result.push(current.join("\n"));
                current.clear();
                in_function = false;
            }
        }
    }
    result
}

fn union_discard_blocks(body: &str) -> Vec<String> {
    let mut blocks = Vec::<String>::new();
    let mut current = String::new();
    for line in body.lines() {
        let code = line.split(';').next().unwrap_or(line).trim();
        let label = code.ends_with(':');
        if label && !current.is_empty() {
            blocks.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

fn union_discard_block_label(block: &str) -> Option<&str> {
    block.lines().next()?.split(';').next()?.trim().strip_suffix(':')
}

#[derive(Debug)]
struct UnionDiscardIrBlock {
    label: String,
    instructions: Vec<String>,
}

fn union_discard_ir_blocks(body: &str) -> Vec<UnionDiscardIrBlock> {
    let mut blocks = Vec::new();
    let mut label = String::from("entry");
    let mut instructions = Vec::new();
    for line in body.lines() {
        let code = line.split(';').next().unwrap_or(line).trim();
        if code.is_empty() || code.starts_with("define ") || code == "}" { continue; }
        if let Some(next_label) = code.strip_suffix(':') {
            if !instructions.is_empty() {
                blocks.push(UnionDiscardIrBlock {
                    label: std::mem::replace(&mut label, next_label.to_string()),
                    instructions: std::mem::take(&mut instructions),
                });
            } else {
                label = next_label.to_string();
            }
        } else {
            instructions.push(code.to_string());
        }
    }
    if !instructions.is_empty() {
        blocks.push(UnionDiscardIrBlock { label, instructions });
    }
    blocks
}

fn union_discard_ir_lhs(line: &str) -> Option<&str> {
    line.split_once(" = ").map(|(lhs, _)| lhs.trim())
}

fn union_discard_ir_rhs(line: &str) -> &str {
    line.split_once(" = ").map_or(line.trim(), |(_, rhs)| rhs.trim())
}

fn union_discard_ir_uses_ssa(text: &str, expected: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' { index += 1; continue; }
        let start = index;
        index += 1;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric()
            || matches!(bytes[index], b'_' | b'.' | b'-' | b'$')) { index += 1; }
        if &text[start..index] == expected { return true; }
    }
    false
}

fn union_discard_ir_conditional_branch(line: &str) -> Option<(String, String, String)> {
    let rest = union_discard_ir_rhs(line).strip_prefix("br i1 ")?;
    let (condition, labels) = rest.split_once(", label %")?;
    let (yes, no) = labels.split_once(", label %")?;
    Some((condition.trim().to_string(), yes.trim().to_string(), no.trim().to_string()))
}

fn union_discard_ir_unconditional_branch(line: &str) -> Option<String> {
    union_discard_ir_rhs(line).strip_prefix("br label %").map(str::to_string)
}

fn union_discard_ir_extract(line: &str, source: &str, index: u64) -> bool {
    union_discard_ir_rhs(line).strip_prefix("extractvalue { i8, i64 } ")
        .and_then(|rest| rest.split_once(", "))
        .is_some_and(|(operand, field)| operand.trim() == source && field.trim().parse::<u64>().ok() == Some(index))
}

fn union_discard_ir_destroy_operand(line: &str) -> Option<String> {
    let args = union_discard_ir_rhs(line)
        .strip_prefix("call void @thaw_promise_destroy(")?
        .split_once(')')?.0.trim();
    args.strip_prefix("ptr ").map(str::to_string)
}

fn union_discard_ir_reachable(blocks: &[UnionDiscardIrBlock], start: &str) -> Vec<String> {
    let mut reached = vec![start.to_string()];
    let mut cursor = 0;
    while cursor < reached.len() {
        let current = &reached[cursor];
        cursor += 1;
        let Some(block) = blocks.iter().find(|block| &block.label == current) else { continue; };
        for line in &block.instructions {
            let next = if let Some((_, yes, no)) = union_discard_ir_conditional_branch(line) {
                Some(vec![yes, no])
            } else {
                union_discard_ir_unconditional_branch(line).map(|label| vec![label])
            };
            if let Some(labels) = next {
                for label in labels {
                    if !reached.contains(&label) { reached.push(label); }
                }
            }
        }
    }
    reached
}

fn assert_union_value_payload_reaches_destroy(
    body: &str,
    union_value: &str,
    expected_tags: &[u64],
) {
    let blocks = union_discard_ir_blocks(body);
    let tag_extracts = blocks.iter().flat_map(|block| block.instructions.iter())
        .filter(|line| union_discard_ir_extract(line, union_value, 0)).collect::<Vec<_>>();
    assert_eq!(tag_extracts.len(), 1, "one exact tag extraction from {union_value}:\n{body}");
    let tag_ssa = union_discard_ir_lhs(tag_extracts[0]).expect("tag extraction has an SSA result").to_string();

    let mut selected_blocks = Vec::<String>::new();
    let mut merge = None::<String>;
    let mut actual_tags = Vec::<u64>::new();
    let mut test_labels = Vec::<String>::new();
    let mut false_edges = Vec::<String>::new();
    for test in &blocks {
        let compares = test.instructions.iter().filter_map(|line| {
            let rhs = union_discard_ir_rhs(line);
            let rest = rhs.strip_prefix("icmp eq i8 ")?;
            let (operand, literal) = rest.split_once(", ")?;
            if operand.trim() != tag_ssa { return None; }
            Some((line, literal.trim().parse::<u64>().ok()?))
        }).collect::<Vec<_>>();
        if compares.is_empty() { continue; }
        assert_eq!(compares.len(), 1, "one selected-tag comparison in {}:\n{body}", test.label);
        let (compare, tag) = compares[0];
        assert!(expected_tags.contains(&tag), "unexpected Promise tag {tag} in {}:\n{body}", test.label);
        actual_tags.push(tag);
        test_labels.push(test.label.clone());
        let compare_ssa = union_discard_ir_lhs(compare).expect("tag comparison has an SSA result");
        let branch = test.instructions.iter().find_map(|line| {
            union_discard_ir_conditional_branch(line)
                .filter(|(condition, _, _)| condition == compare_ssa)
        }).unwrap_or_else(|| panic!("tag {tag} branch condition must be exact comparison SSA {compare_ssa}:\n{body}"));
        assert_eq!(test.instructions.iter().filter(|line|
            union_discard_ir_conditional_branch(line).is_some()).count(), 1,
            "tag test block has only its selected-vs-next conditional:\n{body}");
        assert_eq!(test.instructions.last().and_then(|line| union_discard_ir_conditional_branch(line))
            .as_ref().map(|(condition, _, _)| condition.as_str()), Some(compare_ssa),
            "tag test terminator branches on the exact comparison result:\n{body}");
        let (yes, no) = (branch.1, branch.2);
        false_edges.push(no.clone());
        assert!(yes.starts_with("discard_union_promise"), "tag {tag} true edge selects Promise block {yes}:\n{body}");
        let selected = blocks.iter().find(|candidate| candidate.label == yes)
            .unwrap_or_else(|| panic!("missing selected block {yes}:\n{body}"));
        let payloads = selected.instructions.iter()
            .filter(|line| union_discard_ir_extract(line, union_value, 1))
            .collect::<Vec<_>>();
        assert_eq!(payloads.len(), 1, "tag {tag} selected block extracts this union's payload once:\n{body}");
        let payload = union_discard_ir_lhs(payloads[0]).expect("payload extraction has an SSA result").to_string();
        let decodes = selected.instructions.iter().filter(|line| {
            let rhs = union_discard_ir_rhs(line);
            rhs.starts_with("inttoptr i64 ")
                && rhs.strip_prefix("inttoptr i64 ").is_some_and(|tail| tail.starts_with(&format!("{payload} to ptr")))
        }).collect::<Vec<_>>();
        assert_eq!(decodes.len(), 1, "tag {tag} selected payload is decoded exactly once:\n{body}");
        let pointer = union_discard_ir_lhs(decodes[0]).expect("decoded pointer has an SSA result").to_string();
        let destroys = selected.instructions.iter().filter(|line|
            union_discard_ir_destroy_operand(line).as_deref() == Some(pointer.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(destroys.len(), 1, "tag {tag} selected block destroys exactly its decoded token {pointer}:\n{body}");
        assert_eq!(selected.instructions.iter().filter(|line|
            union_discard_ir_destroy_operand(line).is_some()).count(), 1,
            "selected tag block has no unrelated second Promise destroy:\n{body}");
        let edge = selected.instructions.last().and_then(|line| union_discard_ir_unconditional_branch(line))
            .unwrap_or_else(|| panic!("selected tag {tag} must end at one live unconditional merge edge:\n{body}"));
        assert_eq!(selected.instructions.iter().filter(|line|
            union_discard_ir_unconditional_branch(line).is_some()).count(), 1,
            "selected tag block has exactly one merge terminator:\n{body}");
        assert!(edge.starts_with("discarded_union_end"), "selected tag {tag} rejoins common merge {edge}:\n{body}");
        if let Some(existing) = &merge { assert_eq!(existing, &edge, "all selected tags share one merge:\n{body}"); }
        merge = Some(edge);
        assert_eq!(blocks.iter().flat_map(|candidate| candidate.instructions.iter())
            .filter(|line| union_discard_ir_destroy_operand(line).as_deref() == Some(pointer.as_str())).count(), 1,
            "selected token {pointer} is destroyed exactly once across the function:\n{body}");
        assert!(!selected_blocks.contains(&yes), "each Promise tag has its own selected block:\n{body}");
        selected_blocks.push(yes);
    }
    assert_eq!(actual_tags, expected_tags, "all Promise tag compares occur in member order:\n{body}");
    assert_eq!(false_edges.len(), test_labels.len());
    for index in 0..test_labels.len().saturating_sub(1) {
        assert_eq!(false_edges[index], test_labels[index + 1],
            "tag false edge reaches the exact next Promise-tag test block:\n{body}");
        assert!(false_edges[index].starts_with("discard_union_next"),
            "intervening false edge is the intended next-test block:\n{body}");
    }
    let merge = merge.expect("at least one selected tag reaches the merge");
    let final_compare = blocks.iter().find(|block| block.label == *test_labels.last().unwrap())
        .expect("final Promise-tag test block exists");
    let final_branch = final_compare.instructions.iter().find_map(|line|
        union_discard_ir_conditional_branch(line).filter(|(condition, _, _)| {
            final_compare.instructions.iter().any(|compare| union_discard_ir_lhs(compare) == Some(condition.as_str())
                && union_discard_ir_rhs(compare).starts_with("icmp eq i8 "))
        })).expect("final compare branches on its result");
    let final_next = false_edges.last().expect("final false edge exists");
    assert_eq!(&final_branch.2, final_next, "final false edge enters its empty next block:\n{body}");
    assert!(final_next.starts_with("discard_union_next"), "final false edge uses the helper's final next block:\n{body}");
    let final_next_block = blocks.iter().find(|block| block.label == *final_next)
        .expect("final unmatched-tag next block exists");
    assert_eq!(final_next_block.instructions.len(), 1,
        "final unmatched/plain next block has no decode or discard instructions:\n{body}");
    assert!(!final_next_block.instructions.iter().any(|line|
        union_discard_ir_extract(line, union_value, 1)
            || union_discard_ir_rhs(line).starts_with("inttoptr i64 ")
            || union_discard_ir_destroy_operand(line).is_some()),
        "final plain-tag next block is empty apart from its branch:\n{body}");
    assert_eq!(final_next_block.instructions.last().and_then(|line| union_discard_ir_unconditional_branch(line)).as_deref(),
        Some(merge.as_str()), "final unmatched/plain edge then reaches the common merge:\n{body}");
    let merge_block = blocks.iter().find(|block| block.label == merge)
        .expect("common discarded_union_end block exists");
    assert!(!merge_block.instructions.iter().any(|line| union_discard_ir_rhs(line) == "unreachable"),
        "common union merge is live:\n{body}");
    let reaches_ret = union_discard_ir_reachable(&blocks, &merge).iter().any(|label|
        blocks.iter().find(|block| block.label.as_str() == label.as_str())
            .is_some_and(|block| block.instructions.iter().any(|line| union_discard_ir_rhs(line).starts_with("ret "))));
    assert!(reaches_ret, "common union merge reaches a normal function return:\n{body}");

    for block in &blocks {
        if selected_blocks.contains(&block.label) { continue; }
        assert!(!block.instructions.iter().any(|line| union_discard_ir_extract(line, union_value, 1)),
            "payload extraction must not happen eagerly outside a selected Promise tag block {}:\n{body}", block.label);
        for line in &block.instructions {
            let rhs = union_discard_ir_rhs(line);
            if let Some(payload) = rhs.strip_prefix("inttoptr i64 ").and_then(|tail| tail.split_once(" to ptr").map(|(p, _)| p.trim())) {
                let selected_payload = selected_blocks.iter().any(|label| blocks.iter()
                    .find(|candidate| &candidate.label == label)
                    .is_some_and(|candidate| candidate.instructions.iter().any(|selected_line|
                        union_discard_ir_extract(selected_line, union_value, 1)
                            && union_discard_ir_lhs(selected_line) == Some(payload))));
                assert!(!selected_payload, "selected opaque payload is never decoded outside its tag block:\n{body}");
            }
        }
    }
}

fn assert_union_call_payload_reaches_destroy(
    body: &str,
    producer: &str,
    expected_tags: &[u64],
) {
    assert_union_result_payload_reaches_destroy(body, Some(producer), expected_tags);
}

fn union_discard_injected_union_value(
    body: &str,
    producer: &str,
    injected_tag: u64,
) -> String {
    let blocks = union_discard_ir_blocks(body);
    let calls = blocks.iter().flat_map(|block| block.instructions.iter()).filter(|line|
        union_discard_ir_rhs(line).contains(&format!("call ptr @{producer}("))).collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "direct injection has one scoped Promise producer call:\n{body}");
    let token = union_discard_ir_lhs(calls[0]).expect("Promise producer has pointer SSA result").to_string();
    let call_block = blocks.iter().find(|block| block.instructions.contains(calls[0]))
        .expect("producer call belongs to a block");
    let exception = call_block.instructions.iter().find_map(|line| union_discard_ir_conditional_branch(line)
        .filter(|(_, yes, no)|
            (yes.starts_with("propagate_exception") && no.starts_with("call_ok"))
                || (no.starts_with("propagate_exception") && yes.starts_with("call_ok"))))
        .unwrap_or_else(|| panic!("direct injection producer checks errors before packing its result:\n{body}"));
    let normal = if exception.1.starts_with("call_ok") { exception.1 } else { exception.2 };
    let normal_reachable = union_discard_ir_reachable(&blocks, &normal);
    let packed = blocks.iter().flat_map(|block| block.instructions.iter()).find(|line|
        union_discard_ir_rhs(line) == format!("ptrtoint ptr {token} to i64"))
        .unwrap_or_else(|| panic!("exact producer token {token} is packed into the union payload:\n{body}"));
    let payload_bits = union_discard_ir_lhs(packed).expect("packed pointer bits have SSA result").to_string();
    let inline_payload = format!(
        "insertvalue {{ i8, i64 }} {{ i8 {injected_tag}, i64 undef }}, i64 {payload_bits}, 1"
    );
    let payload_inserts = blocks.iter().flat_map(|block| block.instructions.iter()).filter(|payload_insert| {
        let rhs = union_discard_ir_rhs(payload_insert);
        rhs == inline_payload || blocks.iter().flat_map(|block| block.instructions.iter()).any(|tag_insert| {
            let tag_rhs = union_discard_ir_rhs(tag_insert);
            if !tag_rhs.starts_with("insertvalue { i8, i64 } undef, i8 ")
                || !tag_rhs.ends_with(&format!(", i8 {injected_tag}, 0")) { return false; }
            union_discard_ir_lhs(tag_insert).is_some_and(|tagged|
                rhs == format!("insertvalue {{ i8, i64 }} {tagged}, i64 {payload_bits}, 1"))
        })
    }).collect::<Vec<_>>();
    assert_eq!(payload_inserts.len(), 1,
        "one matching explicit or folded tag {injected_tag} carries the exact producer pointer bits:\n{body}");
    let union_insert = payload_inserts[0];
    let union_block = blocks.iter().find(|block| block.instructions.contains(union_insert))
        .expect("tagged union payload construction belongs to a normal block");
    assert!(normal_reachable.contains(&union_block.label),
        "producer token is injected only after its normal call_ok continuation:\n{body}");
    let union_value = union_discard_ir_lhs(union_insert).expect("payload insert has final union SSA value").to_string();
    union_value
}

fn assert_union_injection_payload_reaches_destroy(
    body: &str,
    producer: &str,
    injected_tag: u64,
    expected_tags: &[u64],
) {
    let union_value = union_discard_injected_union_value(body, producer, injected_tag);
    assert_union_value_payload_reaches_destroy(body, &union_value, expected_tags);
}

fn assert_union_result_payload_reaches_destroy(
    body: &str,
    producer: Option<&str>,
    expected_tags: &[u64],
) {
    let blocks = union_discard_ir_blocks(body);
    let calls = blocks.iter().flat_map(|block| block.instructions.iter())
        .filter(|line| {
            let rhs = union_discard_ir_rhs(line);
            rhs.starts_with("call { i8, i64 } ")
                && producer.is_none_or(|name| rhs.contains(&format!("@{name}(")))
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "one exact scoped native/adapter union result call {producer:?}:\n{body}");
    let call_line = calls[0];
    let call_value = union_discard_ir_lhs(call_line).expect("union call result is an SSA value").to_string();
    let (call_block, _) = blocks.iter().enumerate().find(|(_, block)| block.instructions.contains(call_line))
        .expect("union call belongs to one block");
    let exception_edge = blocks[call_block].instructions.iter().find_map(|line|
        union_discard_ir_conditional_branch(line).filter(|(_, yes, no)|
            (yes.starts_with("propagate_exception") && no.starts_with("call_ok"))
                || (no.starts_with("propagate_exception") && yes.starts_with("call_ok"))))
        .unwrap_or_else(|| panic!("union call's exception check has a distinct call_ok normal successor:\n{body}"));
    let normal = if exception_edge.1.starts_with("call_ok") { exception_edge.1 } else { exception_edge.2 };
    let tag_line = blocks.iter().flat_map(|block| block.instructions.iter())
        .find(|line| union_discard_ir_extract(line, &call_value, 0))
        .unwrap_or_else(|| panic!("missing exact tag extraction from {call_value}:\n{body}"));
    let tag_block = blocks.iter().find(|block| block.instructions.contains(tag_line)).unwrap();
    assert!(tag_block.label.starts_with("call_ok") || union_discard_ir_reachable(&blocks, &normal).contains(&tag_block.label),
        "tag extraction follows the native call's normal continuation:\n{body}");
    assert_union_value_payload_reaches_destroy(body, &call_value, expected_tags);
}

fn union_discard_destroy_call_lines(body: &str) -> Vec<&str> {
    body.lines().filter(|line| line.contains("call void @thaw_promise_destroy(")).collect()
}

fn union_discard_ir_load_pointer_operand(line: &str, value_type: &str) -> Option<String> {
    let prefix = format!("load {value_type}, ptr ");
    union_discard_ir_rhs(line).strip_prefix(&prefix)
        .map(|rest| rest.split(',').next().unwrap_or(rest).trim().to_string())
}

fn union_discard_ir_store_pointer_operands(line: &str) -> Option<(String, String)> {
    let rest = union_discard_ir_rhs(line).strip_prefix("store ptr ")?;
    let (value, destination) = rest.split_once(", ptr ")?;
    Some((value.trim().to_string(), destination.split(',').next()?.trim().to_string()))
}

fn union_discard_ir_pointer_parameters(body: &str) -> Vec<String> {
    let signature = body.lines().find(|line| line.trim_start().starts_with("define "))
        .expect("fixture function definition has a signature");
    let arguments = signature.split_once('@').and_then(|(_, rest)| rest.split_once('(').map(|(_, args)| args))
        .and_then(|args| args.split_once(')').map(|(args, _)| args))
        .expect("fixture signature has a parameter list");
    arguments.split(',').filter_map(|argument| {
        let mut words = argument.split_whitespace();
        (words.next()? == "ptr").then(|| words.rfind(|word| word.starts_with('%')).map(str::to_string)).flatten()
    }).collect()
}

fn assert_borrowed_promise_cleanup_flag_is_false_initialized(body: &str, parameter: &str) {
    let blocks = union_discard_ir_blocks(body);
    let slot = format!("%{parameter}_cell");
    let incoming = blocks.iter().flat_map(|block| block.instructions.iter()).find_map(|line|
        union_discard_ir_store_pointer_operands(line)
            .filter(|(_, destination)| destination == &slot)
            .map(|(value, _)| value))
        .unwrap_or_else(|| panic!("named Promise local slot {slot} stores one incoming pointer parameter:\n{body}"));
    let pointer_parameters = union_discard_ir_pointer_parameters(body);
    assert_eq!(pointer_parameters.len(), 1,
        "this fixture has one pointer-typed function parameter, identified from its actual signature:\n{body}");
    assert_eq!(incoming, pointer_parameters[0],
        "the exact named Promise slot receives the function's pointer parameter:\n{body}");
    let (flag_block, flag) = blocks.iter().find_map(|block| block.instructions.iter().find_map(|line| {
        let rhs = union_discard_ir_rhs(line);
        rhs.strip_prefix("store i1 false, ptr ")
            .map(|rest| (block.label.clone(), rest.split(',').next().unwrap_or(rest).trim().to_string()))
    })).unwrap_or_else(|| panic!("borrowed owner flag is initialized false:\n{body}"));
    assert_eq!(flag_block.as_str(), "entry", "borrowed owner flag starts false in function entry:\n{body}");
    let owned = blocks.iter().flat_map(|block| block.instructions.iter()).find_map(|line| {
        (union_discard_ir_load_pointer_operand(line, "i1").as_deref() == Some(flag.as_str()))
            .then(|| union_discard_ir_lhs(line))
            .flatten()
            .map(str::to_string)
    }).unwrap_or_else(|| panic!("cleanup loads that exact false-initialized owner flag {flag}:\n{body}"));
    let guard = blocks.iter().find_map(|block| block.instructions.iter().find_map(|line| {
        union_discard_ir_conditional_branch(line).filter(|(condition, yes, no)|
            condition == &owned
                && yes.starts_with("release_stack_promise")
                && no.starts_with("stack_promise_cleanup_next"))
                .map(|(_, yes, _)| yes.clone())
    })).unwrap_or_else(|| panic!("only a true owner flag enters lexical Promise release:\n{body}"));
    let release = blocks.iter().find(|block| block.label == guard)
        .unwrap_or_else(|| panic!("flag-guarded release block {guard} exists:\n{body}"));
    let loaded = release.instructions.iter().find_map(|line|
        (union_discard_ir_load_pointer_operand(line, "ptr").as_deref() == Some(slot.as_str()))
            .then(|| union_discard_ir_lhs(line))
            .flatten()
            .map(str::to_string)
    ).unwrap_or_else(|| panic!("release reloads the borrowed parameter's exact owner slot {slot}:\n{body}"));
    assert_eq!(release.instructions.iter().filter_map(|line| union_discard_ir_destroy_operand(line))
        .collect::<Vec<_>>(), vec![loaded], "guarded lexical cleanup destroys only the slot's current token:\n{body}");
}

fn assert_direct_wrapper_release(body: &str, wrapper_value: &str) {
    let calls = union_discard_destroy_call_lines(body);
    assert_eq!(calls.len(), 1, "owned direct Promise wrapper has one isolated release:\n{body}");
    let expected = format!("%{wrapper_value}");
    assert_eq!(union_discard_ir_destroy_operand(calls[0]).as_deref(), Some(expected.as_str()),
        "destroy operand must be the exact selected Value extraction %{wrapper_value}:\n{body}");
    assert!(!body.contains("call i8 @thaw_promise_retain("),
        "discarded owned wrapper must not acquire a second token:\n{body}");
}

fn assert_union_wrapper_release(body: &str, wrapper_value: &str, tags: &[u64]) {
    assert_union_value_payload_reaches_destroy(body, &format!("%{wrapper_value}"), tags);
    assert!(!body.contains("call i8 @thaw_promise_retain("),
        "discarded owned union wrapper must not retain:\n{body}");
}

fn assert_no_wrapper_discard_release(body: &str, extracted_value: &str) {
    assert!(union_discard_destroy_call_lines(body).iter().all(|line|
        union_discard_ir_destroy_operand(line).is_some_and(|operand| operand != format!("%{extracted_value}"))),
        "no discard destructor may consume the exact borrowed/plain Value extraction %{extracted_value}:\n{body}");
    let blocks = union_discard_ir_blocks(body);
    for block in blocks.iter().filter(|block| block.instructions.iter().any(|line|
        union_discard_ir_destroy_operand(line).is_some())) {
        assert!(block.label.starts_with("release_stack_promise"),
            "borrowed/plain wrapper has no discard release outside guarded lexical cleanup:\n{body}");
    }
    assert!(!body.contains("call i8 @thaw_promise_retain("),
        "discard must not retain a borrowed/plain wrapper:\n{body}");
}

fn assert_owned_conditional_arm_uses_its_producer(body: &str, arm_label: &str) {
    let blocks = union_discard_ir_blocks(body);
    let block = blocks.iter().find(|block| block.label == arm_label)
        .unwrap_or_else(|| panic!("missing owned conditional arm {arm_label}:\n{body}"));
    let calls = block.instructions.iter().filter(|line|
        union_discard_ir_rhs(line).contains("call ptr @owned_wrapper_promise(")).collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "owned selected arm evaluates its factory exactly once:\n{body}");
    let token = union_discard_ir_lhs(calls[0]).expect("factory call returns owned token").to_string();
    assert!(!block.instructions.iter().any(|line|
        union_discard_ir_destroy_operand(line).as_deref() == Some(token.as_str())),
        "factory result is not destroyed before its normal call continuation:\n{body}");
    let exception = block.instructions.iter().find_map(|line| union_discard_ir_conditional_branch(line)
        .filter(|(_, yes, no)|
            (yes.starts_with("propagate_exception") && no.starts_with("call_ok"))
                || (no.starts_with("propagate_exception") && yes.starts_with("call_ok"))))
        .unwrap_or_else(|| panic!("factory call checks pending exception before wrapper release:\n{body}"));
    let call_ok = if exception.1.starts_with("call_ok") { exception.1 } else { exception.2 };
    let normal_reachable = union_discard_ir_reachable(&blocks, &call_ok);
    let inserted = blocks.iter().filter(|candidate| normal_reachable.contains(&candidate.label))
        .flat_map(|candidate| candidate.instructions.iter())
        .filter(|line| union_discard_ir_rhs(line).starts_with("insertvalue {")
            && union_discard_ir_uses_ssa(union_discard_ir_rhs(line), &token))
        .collect::<Vec<_>>();
    assert_eq!(inserted.len(), 1, "normal continuation constructs its matching Some wrapper from {token}:\n{body}");
    let wrapper = union_discard_ir_lhs(inserted[0]).expect("Some construction has an SSA value").to_string();
    let extracted = blocks.iter().filter(|candidate| normal_reachable.contains(&candidate.label))
        .flat_map(|candidate| candidate.instructions.iter())
        .filter(|line| union_discard_ir_rhs(line).starts_with("extractvalue {")
            && union_discard_ir_uses_ssa(union_discard_ir_rhs(line), &wrapper)
            && union_discard_ir_rhs(line).ends_with(", 1"))
        .collect::<Vec<_>>();
    assert_eq!(extracted.len(), 1, "normal continuation extracts the selected Some payload from {wrapper} once:\n{body}");
    let extracted = extracted[0];
    let value = union_discard_ir_lhs(extracted).expect("Some payload extraction has SSA result").to_string();
    assert_ne!(value, token, "release consumes the explicit wrapper extraction, not raw factory SSA");
    let destroys = blocks.iter().filter(|candidate| normal_reachable.contains(&candidate.label))
        .flat_map(|candidate| candidate.instructions.iter())
        .filter(|line| union_discard_ir_destroy_operand(line).as_deref() == Some(value.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(destroys.len(), 1, "normal Some continuation destroys exactly its extracted owned payload:\n{body}");
    assert_eq!(union_discard_destroy_call_lines(body).iter().filter(|line|
        union_discard_ir_destroy_operand(line).as_deref() == Some(value.as_str())).count(), 1,
        "the exact extracted owned token is destroyed only once across this function:\n{body}");
    assert_eq!(union_discard_destroy_call_lines(body).iter().filter(|line|
        union_discard_ir_destroy_operand(line).as_deref() == Some(token.as_str())).count(), 0,
        "wrapper release never consumes the raw factory result:\n{body}");
}

fn assert_conditional_arm_edges(body: &str, owned_arm: &str, borrowed_arm: &str) {
    let branches = union_discard_ir_blocks(body).iter().flat_map(|block| block.instructions.iter())
        .filter_map(|line| union_discard_ir_conditional_branch(line))
        .filter(|(_, yes, no)|
            (yes.starts_with("discard_promise_then") && no.starts_with("discard_promise_else"))
                || (yes.starts_with("discard_promise_else") && no.starts_with("discard_promise_then")))
        .collect::<Vec<_>>();
    assert_eq!(branches.len(), 1, "one source conditional links its true/false arms:\n{body}");
    let (expected_yes, expected_no) = if owned_arm.starts_with("discard_promise_then") {
        (owned_arm, borrowed_arm)
    } else {
        (borrowed_arm, owned_arm)
    };
    assert!(branches[0].1.starts_with(expected_yes) && branches[0].2.starts_with(expected_no),
        "conditional true/false edges select the corresponding owned/borrowed source arms:\n{body}");
}

fn assert_nested_owned_plain_borrowed_paths(body: &str, producer: &str) {
    let blocks = union_discard_ir_blocks(body);
    let branches = blocks.iter().filter_map(|block| {
        block.instructions.last().and_then(|line| union_discard_ir_conditional_branch(line))
            .filter(|(_, yes, no)| yes.starts_with("discard_promise_then")
                && no.starts_with("discard_promise_else"))
            .map(|(_, yes, no)| (block.label.clone(), yes, no))
    }).collect::<Vec<_>>();
    assert_eq!(branches.len(), 2, "nested fixture has outer and inner selected-arm branches:\n{body}");
    let call_block = blocks.iter().find(|block| block.instructions.iter().any(|line|
        union_discard_ir_rhs(line).contains(&format!("call ptr @{producer}("))))
        .unwrap_or_else(|| panic!("nested owned leaf calls {producer}:\n{body}"));
    let parents = branches.iter().filter_map(|outer| {
        let outer_yes = union_discard_ir_reachable(&blocks, &outer.1);
        let outer_no = union_discard_ir_reachable(&blocks, &outer.2);
        branches.iter().find(|inner| inner.0 != outer.0
            && outer_yes.contains(&inner.0) && !outer_no.contains(&inner.0))
            .map(|inner| (outer, inner, outer_yes, outer_no))
    }).collect::<Vec<_>>();
    assert_eq!(parents.len(), 1, "outer true edge alone reaches the nested inner selection:\n{body}");
    let (outer, inner, outer_yes, outer_no) = &parents[0];
    let inner_yes = union_discard_ir_reachable(&blocks, &inner.1);
    let inner_no = union_discard_ir_reachable(&blocks, &inner.2);
    assert!(outer_yes.contains(&call_block.label) && inner_yes.contains(&call_block.label),
        "the named owned leaf is reachable only through outer-true then inner-true:\n{body}");
    assert!(!outer_no.contains(&call_block.label) && !inner_no.contains(&call_block.label),
        "borrowed and plain leaf paths cannot reach the owned producer:\n{body}");
    for (label, kind) in [(&inner.2, "plain"), (&outer.2, "borrowed")] {
        let leaf = blocks.iter().find(|block| block.label == *label)
            .unwrap_or_else(|| panic!("{kind} leaf block {label} exists:\n{body}"));
        assert!(!leaf.instructions.iter().any(|line|
            union_discard_ir_rhs(line).starts_with("inttoptr i64 ")
                || union_discard_ir_destroy_operand(line).is_some()),
            "the {kind} leaf has no Promise decode or discard destructor:\n{body}");
    }
}

fn assert_adapter_returns_call_result(body: &str, target: &str) {
    let blocks = union_discard_ir_blocks(body);
    let call = blocks.iter().flat_map(|block| block.instructions.iter()).find(|line|
        union_discard_ir_rhs(line).contains(&format!("call {{ i8, i64 }} @{target}(")))
        .unwrap_or_else(|| panic!("adapter does not call its exact forwarding target {target}:\n{body}"));
    let result = union_discard_ir_lhs(call).expect("adapter call has result SSA");
    assert!(blocks.iter().flat_map(|block| block.instructions.iter()).any(|line|
        union_discard_ir_rhs(line) == format!("ret {{ i8, i64 }} {result}")),
        "adapter must return the exact SSA result {result} from {target}:\n{body}");
    assert!(!body.contains("@thaw_promise_retain(") && !body.contains("@thaw_promise_destroy("),
        "forwarding adapter neither acquires nor releases the transferred token:\n{body}");
}

fn assert_pending_exception_continuation_precedes_union_discard(body: &str) {
    assert!(body.lines().any(|line|
        line.contains("br i1") && line.contains("has_pending_exception")
            && line.contains("label %propagate_exception") && line.contains("label %call_ok")),
        "call failure must branch away before normal union discard:\n{body}");
    let call_ok = union_discard_blocks(body).into_iter()
        .find(|block| union_discard_block_label(block) == Some("call_ok"))
        .expect("call_ok continuation block exists");
    assert!(call_ok.contains("discarded_union_tag") || call_ok.contains("discarded_union_is_promise"),
        "selected union dispatch is emitted only in the normal call continuation:\n{body}");
}

#[test]
fn discarded_native_promise_union_owned_and_plain_arms() {
    let flat_members = vec![
        HirType::Promise(Box::new(HirType::F64)),
        HirType::F64,
    ];
    let three_members = vec![
        HirType::Promise(Box::new(HirType::F64)),
        HirType::Str,
        HirType::Promise(Box::new(HirType::I64)),
    ];
    let _flat = HirType::Union(flat_members.clone());
    let three = HirType::Union(three_members.clone());

    let make_flat = union_discard_union_producer(
        "owned_native_flat_union",
        &flat_members,
        HirExpr::UnionInject(
            Box::new(union_discard_call("owned_native_promise_f64")),
            0,
            flat_members.clone(),
        ),
    );
    let make_three = union_discard_function(
        "owned_native_three_union",
        vec![HirParam { name: "choose_i64".into(), ty: HirType::Bool }],
        three.clone(),
        vec![HirStmt::Return(Some(HirExpr::Conditional(
            Box::new(HirExpr::Var("choose_i64".into())),
            Box::new(HirExpr::UnionInject(
                Box::new(union_discard_call("owned_native_promise_f64")),
                0,
                three_members.clone(),
            )),
            Box::new(HirExpr::UnionInject(
                Box::new(union_discard_call("owned_native_promise_i64")),
                2,
                three_members.clone(),
            )),
            three.clone(),
        )))],
    );
    let make_flat_f64 = union_discard_union_producer(
        "plain_native_flat_f64_union",
        &flat_members,
        HirExpr::UnionInject(
            Box::new(HirExpr::Lit(HirLit::F64(f64::from_bits(0x0000_0000_0000_1234)))),
            1,
            flat_members.clone(),
        ),
    );
    let make_three_str = union_discard_union_producer(
        "plain_native_three_str_union",
        &three_members,
        HirExpr::UnionInject(
            Box::new(HirExpr::Lit(HirLit::Str("opaque pointer-looking payload".into()))),
            1,
            three_members.clone(),
        ),
    );

    let functions = vec![
        union_discard_promise_producer("owned_native_promise_f64", HirType::F64),
        union_discard_promise_producer("owned_native_promise_i64", HirType::I64),
        make_flat,
        make_three,
        make_flat_f64,
        make_three_str,
        union_discard_function(
            "discard_native_flat_union_call", vec![], HirType::Void,
            vec![HirStmt::Expr(union_discard_call("owned_native_flat_union")), HirStmt::Return(None)],
        ),
        union_discard_function(
            "discard_native_three_union_call",
            vec![HirParam { name: "choose_i64".into(), ty: HirType::Bool }],
            HirType::Void,
            vec![
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("owned_native_three_union".into())),
                    vec![HirExpr::Var("choose_i64".into())],
                )),
                HirStmt::Return(None),
            ],
        ),
        union_discard_function(
            "discard_plain_native_flat_call", vec![], HirType::Void,
            vec![HirStmt::Expr(union_discard_call("plain_native_flat_f64_union")), HirStmt::Return(None)],
        ),
        union_discard_function(
            "discard_plain_native_three_call", vec![], HirType::Void,
            vec![HirStmt::Expr(union_discard_call("plain_native_three_str_union")), HirStmt::Return(None)],
        ),
        union_discard_function(
            "discard_direct_flat_injection", vec![], HirType::Void,
            vec![
                HirStmt::Expr(HirExpr::UnionInject(
                    Box::new(union_discard_call("owned_native_promise_f64")),
                    0,
                    flat_members.clone(),
                )),
                HirStmt::Return(None),
            ],
        ),
        union_discard_function(
            "discard_direct_three_tag0_injection", vec![], HirType::Void,
            vec![
                HirStmt::Expr(HirExpr::UnionInject(
                    Box::new(union_discard_call("owned_native_promise_f64")),
                    0,
                    three_members.clone(),
                )),
                HirStmt::Return(None),
            ],
        ),
        union_discard_function(
            "discard_direct_three_tag2_injection", vec![], HirType::Void,
            vec![
                HirStmt::Expr(HirExpr::UnionInject(
                    Box::new(union_discard_call("owned_native_promise_i64")),
                    2,
                    three_members.clone(),
                )),
                HirStmt::Return(None),
            ],
        ),
        union_discard_function(
            "discard_direct_plain_f64_injection", vec![], HirType::Void,
            vec![
                HirStmt::Expr(HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(HirLit::F64(f64::from_bits(0x0000_0000_0000_4321)))),
                    1,
                    flat_members.clone(),
                )),
                HirStmt::Return(None),
            ],
        ),
        union_discard_function(
            "discard_direct_plain_str_injection", vec![], HirType::Void,
            vec![
                HirStmt::Expr(HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(HirLit::Str("plain pointer-like bits".into()))),
                    1,
                    three_members.clone(),
                )),
                HirStmt::Return(None),
            ],
        ),
    ];
    let program = HirProgram { functions, ..HirProgram::default() };
    let ir = compile_union_discard_program(&program, "discarded_native_union_owned_plain_split");

    // Each consumer is inspected separately. The dynamic producer result
    // remains the sole source for the tag and payload SSA used by destruction.
    let flat_body = union_discard_function_body(&ir, "discard_native_flat_union_call");
    assert_union_call_payload_reaches_destroy(&flat_body, "owned_native_flat_union", &[0]);
    assert!(!flat_body.contains("call i8 @thaw_promise_retain("), "{flat_body}");
    assert!(flat_body.contains("ret void"), "flat discard must leave a live continuation:\n{flat_body}");

    let three_body = union_discard_function_body(&ir, "discard_native_three_union_call");
    assert_union_call_payload_reaches_destroy(&three_body, "owned_native_three_union", &[0, 2]);
    assert!(!three_body.contains("call i8 @thaw_promise_retain("), "{three_body}");

    // Explicit injection consumers exercise the same source-token identity
    // through the selected payload round trip, with both three-member tags.
    for (function, promise_factory, injected_tag, tags) in [
        ("discard_direct_flat_injection", "owned_native_promise_f64", 0, vec![0]),
        ("discard_direct_three_tag0_injection", "owned_native_promise_f64", 0, vec![0, 2]),
        ("discard_direct_three_tag2_injection", "owned_native_promise_i64", 2, vec![0, 2]),
    ] {
        let body = union_discard_function_body(&ir, function);
        assert_union_injection_payload_reaches_destroy(&body, promise_factory, injected_tag, &tags);
        assert!(!body.contains("call i8 @thaw_promise_retain("), "{body}");
    }

    // The plain producers return opaque union calls, so their static dispatch
    // code may contain Promise-only blocks; those blocks must be selected by
    // Promise tags and the sibling JIT controls prove plain tags do not execute
    // any Promise operation or pointer decoder.
    let plain_f64 = union_discard_function_body(&ir, "discard_plain_native_flat_call");
    assert!(plain_f64.contains("@plain_native_flat_f64_union("), "{plain_f64}");
    assert_union_call_payload_reaches_destroy(&plain_f64, "plain_native_flat_f64_union", &[0]);
    let plain_str = union_discard_function_body(&ir, "discard_plain_native_three_call");
    assert!(plain_str.contains("@plain_native_three_str_union("), "{plain_str}");
    assert_union_call_payload_reaches_destroy(&plain_str, "plain_native_three_str_union", &[0, 2]);

    for name in [
        "discard_direct_plain_f64_injection",
        "discard_direct_plain_str_injection",
    ] {
        let body = union_discard_function_body(&ir, name);
        assert!(union_discard_destroy_call_lines(&body).is_empty(),
            "direct plain injection does not enter a Promise destructor path:\n{body}");
        assert!(!body.contains("inttoptr i64"),
            "direct plain payload bits are never decoded as a Promise pointer:\n{body}");
    }
}

#[test]
fn discarded_native_promise_union_borrowed_aliases() {
    let flat_members = vec![
        HirType::Promise(Box::new(HirType::F64)),
        HirType::F64,
    ];
    let flat = HirType::Union(flat_members.clone());
    let promise = HirType::Promise(Box::new(HirType::F64));
    let program = HirProgram {
        functions: vec![
            union_discard_function(
                "discard_borrowed_union_alias",
                vec![HirParam { name: "borrowed_union".into(), ty: flat.clone() }],
                HirType::Void,
                vec![
                    HirStmt::Expr(HirExpr::Var("borrowed_union".into())),
                    HirStmt::Return(None),
                ],
            ),
            union_discard_function(
                "discard_borrowed_exact_promise_alias",
                vec![HirParam { name: "borrowed_promise".into(), ty: promise.clone() }],
                HirType::Void,
                vec![
                    HirStmt::Expr(HirExpr::Var("borrowed_promise".into())),
                    HirStmt::Return(None),
                ],
            ),
            union_discard_function(
                "discard_borrowed_union_projection_after_tag_guard",
                vec![HirParam { name: "borrowed_union".into(), ty: flat }],
                HirType::Void,
                vec![
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(
                                Box::new(HirExpr::Var("borrowed_union".into())),
                                flat_members.clone(),
                            )),
                            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                        ),
                        vec![HirStmt::Expr(HirExpr::UnionValue(
                            Box::new(HirExpr::Var("borrowed_union".into())),
                            0,
                            flat_members,
                        ))],
                        vec![],
                    ),
                    HirStmt::Return(None),
                ],
            ),
        ],
        ..HirProgram::default()
    };
    let ir = compile_union_discard_program(&program, "discarded_native_union_borrowed_aliases");

    let union_alias = union_discard_function_body(&ir, "discard_borrowed_union_alias");
    assert!(!union_alias.contains("@thaw_promise_destroy("), "a borrowed union alias has no owner cleanup or discard release:\n{union_alias}");
    assert!(!union_alias.contains("@thaw_promise_retain("), "discard must not acquire a borrowed union alias:\n{union_alias}");

    let exact_alias = union_discard_function_body(&ir, "discard_borrowed_exact_promise_alias");
    assert!(!exact_alias.contains("discard_union_promise"), "exact Promise alias is not decoded as a union:\n{exact_alias}");
    assert!(!exact_alias.contains("call i8 @thaw_promise_retain("), "discard must not acquire the alias:\n{exact_alias}");
    let cleanup_blocks = union_discard_blocks(&exact_alias).into_iter()
        .filter(|block| block.contains("@thaw_promise_destroy("))
        .collect::<Vec<_>>();
    assert!(!cleanup_blocks.is_empty(), "the existing lexical owner-table cleanup remains present:\n{exact_alias}");
    assert!(cleanup_blocks.iter().all(|block|
        union_discard_block_label(block).is_some_and(|label| label.starts_with("release_stack_promise"))),
        "only flag-guarded lexical cleanup may destroy a borrowed Promise parameter:\n{exact_alias}");
    assert_borrowed_promise_cleanup_flag_is_false_initialized(&exact_alias, "borrowed_promise");

    let projection = union_discard_function_body(&ir, "discard_borrowed_union_projection_after_tag_guard");
    assert!(!projection.contains("@thaw_promise_destroy("), "tag-guarded UnionValue projection stays borrowed:\n{projection}");
    assert!(!projection.contains("@thaw_promise_retain("), "discarded projection acquires no alias:\n{projection}");
    assert!(projection.contains("union_tag"), "projection is protected by the matching runtime tag:\n{projection}");

    // Native integration liveness: the original owner remains usable after a
    // borrowed union alias is discarded. This is a later user-run control.
    let source = r#"
        function main(): void {
            const owner: Promise<number> = Promise.resolve(42);
            const borrowed: Promise<number> | number = owner;
            borrowed;
            owner.then((value: number): void => { console.log(value); });
        }
    "#;
    assert_eq!(compile_and_run(source, "borrowed_promise_union_live_after_discard"), "42\n");
}

#[test]
fn discarded_native_promise_union_selected_wrappers() {
    let promise = HirType::Promise(Box::new(HirType::F64));
    let flat_members = vec![promise.clone(), HirType::F64];
    let flat = HirType::Union(flat_members.clone());
    let three_members = union_discard_three_members();
    let mut functions = vec![
        union_discard_promise_producer("owned_wrapper_promise", HirType::F64),
    ];
    let family_specs = [
        ("optional", UnionDiscardValueFamily::Optional, "optional_value"),
        ("nullable", UnionDiscardValueFamily::Nullable, "nullable_value"),
        ("nullish", UnionDiscardValueFamily::Nullish, "nullish_value"),
    ];

    for (family_name, family, _value_name) in family_specs {
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_owned_promise_some"),
            vec![],
            union_discard_some_value(
                family, union_discard_call("owned_wrapper_promise"), &promise,
            ),
        ));
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_borrowed_promise_some"),
            vec![HirParam { name: "borrowed_promise".into(), ty: promise.clone() }],
            union_discard_some_value(
                family, HirExpr::Var("borrowed_promise".into()), &promise,
            ),
        ));
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_owned_union_some"),
            vec![],
            union_discard_some_value(
                family,
                HirExpr::UnionInject(
                    Box::new(union_discard_call("owned_wrapper_promise")),
                    0,
                    flat_members.clone(),
                ),
                &flat,
            ),
        ));
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_borrowed_union_some"),
            vec![HirParam { name: "borrowed_promise".into(), ty: promise.clone() }],
            union_discard_some_value(
                family,
                HirExpr::UnionInject(
                    Box::new(HirExpr::Var("borrowed_promise".into())),
                    0,
                    flat_members.clone(),
                ),
                &flat,
            ),
        ));
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_plain_union_some"),
            vec![],
            union_discard_some_value(
                family,
                HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(HirLit::Str("plain pointer-looking member".into()))),
                    1,
                    three_members.clone(),
                ),
                &HirType::Union(three_members.clone()),
            ),
        ));
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_plain_f64_some"),
            vec![],
            union_discard_some_value(
                family,
                HirExpr::Lit(HirLit::F64(f64::from_bits(0x0000_0000_0000_1234))),
                &HirType::F64,
            ),
        ));
        for (selection, conditional) in [
            ("owned_then_borrowed", HirExpr::Conditional(
                Box::new(HirExpr::Var("choose_owned".into())),
                Box::new(union_discard_call("owned_wrapper_promise")),
                Box::new(HirExpr::Var("borrowed_promise".into())),
                promise.clone(),
            )),
            ("borrowed_then_owned", HirExpr::Conditional(
                Box::new(HirExpr::Var("choose_owned".into())),
                Box::new(HirExpr::Var("borrowed_promise".into())),
                Box::new(union_discard_call("owned_wrapper_promise")),
                promise.clone(),
            )),
        ] {
            functions.push(union_discard_expression_probe(
                &format!("discard_{family_name}_{selection}_some"),
                vec![
                    HirParam { name: "choose_owned".into(), ty: HirType::Bool },
                    HirParam { name: "borrowed_promise".into(), ty: promise.clone() },
                ],
                union_discard_some_value(family, conditional, &promise),
            ));
        }
        // The Some proof must also survive an exact matching tagged-union
        // inject/extract round trip between TypedClosure and Value.
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_owned_promise_some_union_roundtrip"),
            vec![],
            union_discard_roundtrip_some_value(
                family, union_discard_call("owned_wrapper_promise"), &promise,
            ),
        ));
        functions.push(union_discard_expression_probe(
            &format!("discard_{family_name}_borrowed_promise_some_union_roundtrip"),
            vec![HirParam { name: "borrowed_promise".into(), ty: promise.clone() }],
            union_discard_roundtrip_some_value(
                family, HirExpr::Var("borrowed_promise".into()), &promise,
            ),
        ));
    }

    let nested = HirExpr::Conditional(
        Box::new(HirExpr::Var("outer".into())),
        Box::new(HirExpr::Conditional(
            Box::new(HirExpr::Var("inner".into())),
            Box::new(HirExpr::UnionInject(
                Box::new(union_discard_call("owned_wrapper_promise")), 0, flat_members.clone(),
            )),
            Box::new(HirExpr::UnionInject(
                Box::new(HirExpr::Lit(HirLit::F64(7.0))), 1, flat_members.clone(),
            )),
            flat.clone(),
        )),
        Box::new(HirExpr::UnionInject(
            Box::new(HirExpr::Var("borrowed_promise".into())), 0, flat_members.clone(),
        )),
        flat.clone(),
    );
    functions.push(union_discard_expression_probe(
        "discard_nested_owned_plain_borrowed_union",
        vec![
            HirParam { name: "outer".into(), ty: HirType::Bool },
            HirParam { name: "inner".into(), ty: HirType::Bool },
            HirParam { name: "borrowed_promise".into(), ty: promise.clone() },
        ],
        nested,
    ));
    functions.push(union_discard_expression_probe(
        "discard_opaque_optional_hold",
        vec![HirParam {
            name: "opaque_optional".into(),
            ty: HirType::Optional(Box::new(promise.clone())),
        }],
        HirExpr::Var("opaque_optional".into()),
    ));

    let ir = compile_union_discard_program(
        &HirProgram { functions, ..HirProgram::default() },
        "discarded_native_union_selected_wrappers",
    );

    for (family_name, _, value_name) in family_specs {
        let owned = union_discard_function_body(
            &ir, &format!("discard_{family_name}_owned_promise_some"),
        );
        assert_direct_wrapper_release(&owned, value_name);

        let borrowed = union_discard_function_body(
            &ir, &format!("discard_{family_name}_borrowed_promise_some"),
        );
        assert_no_wrapper_discard_release(&borrowed, value_name);
        assert_borrowed_promise_cleanup_flag_is_false_initialized(&borrowed, "borrowed_promise");

        let owned_union = union_discard_function_body(
            &ir, &format!("discard_{family_name}_owned_union_some"),
        );
        assert_union_wrapper_release(&owned_union, value_name, &[0]);

        let borrowed_union = union_discard_function_body(
            &ir, &format!("discard_{family_name}_borrowed_union_some"),
        );
        assert_no_wrapper_discard_release(&borrowed_union, value_name);
        assert_borrowed_promise_cleanup_flag_is_false_initialized(&borrowed_union, "borrowed_promise");

        let plain_union = union_discard_function_body(
            &ir, &format!("discard_{family_name}_plain_union_some"),
        );
        assert_no_wrapper_discard_release(&plain_union, value_name);
        assert!(!plain_union.contains("inttoptr i64"),
            "plain Str payload bits must never be decoded as a Promise:\n{plain_union}");

        let plain_f64 = union_discard_function_body(
            &ir, &format!("discard_{family_name}_plain_f64_some"),
        );
        assert_no_wrapper_discard_release(&plain_f64, value_name);

        for (selection, owned_arm, borrowed_arm) in [
            ("owned_then_borrowed", "discard_promise_then", "discard_promise_else"),
            ("borrowed_then_owned", "discard_promise_else", "discard_promise_then"),
        ] {
            let conditional = union_discard_function_body(
                &ir, &format!("discard_{family_name}_{selection}_some"),
            );
            assert_conditional_arm_edges(&conditional, owned_arm, borrowed_arm);
            assert_owned_conditional_arm_uses_its_producer(&conditional, owned_arm);
            let borrowed = union_discard_blocks(&conditional).into_iter()
                .find(|block| union_discard_block_label(block) == Some(borrowed_arm))
                .expect("borrowed conditional arm emitted");
            assert!(!borrowed.contains("@thaw_promise_destroy("),
                "borrowed selected arm receives zero discard destroy calls:\n{borrowed}");
            assert!(!borrowed.contains("@thaw_promise_retain("),
                "borrowed selected arm receives zero discard retain calls:\n{borrowed}");
            assert_borrowed_promise_cleanup_flag_is_false_initialized(&conditional, "borrowed_promise");
        }

        let owned_roundtrip = union_discard_function_body(
            &ir, &format!("discard_{family_name}_owned_promise_some_union_roundtrip"),
        );
        assert_direct_wrapper_release(&owned_roundtrip, value_name);
        let borrowed_roundtrip = union_discard_function_body(
            &ir, &format!("discard_{family_name}_borrowed_promise_some_union_roundtrip"),
        );
        assert_no_wrapper_discard_release(&borrowed_roundtrip, value_name);
        assert_borrowed_promise_cleanup_flag_is_false_initialized(&borrowed_roundtrip, "borrowed_promise");
    }

    let nested = union_discard_function_body(&ir, "discard_nested_owned_plain_borrowed_union");
    assert_union_injection_payload_reaches_destroy(&nested, "owned_wrapper_promise", 0, &[0]);
    assert_nested_owned_plain_borrowed_paths(&nested, "owned_wrapper_promise");
    assert!(nested.contains("discard_promise_then") && nested.contains("discard_promise_else"),
        "nested and reversed selections are represented by explicit CFG branches:\n{nested}");
    assert!(!nested.contains("call i8 @thaw_promise_retain("),
        "nested discard does not retain borrowed or plain leaves:\n{nested}");
    assert_borrowed_promise_cleanup_flag_is_false_initialized(&nested, "borrowed_promise");

    let opaque = union_discard_function_body(&ir, "discard_opaque_optional_hold");
    assert!(union_discard_destroy_call_lines(&opaque).is_empty(),
        "opaque optional input has no proven explicit Some payload:\n{opaque}");
}

#[test]
fn discarded_native_promise_union_void_lambda_and_adapters() {
    let promise = HirType::Promise(Box::new(HirType::F64));
    let flat_members = vec![promise.clone(), HirType::F64];
    let flat = HirType::Union(flat_members.clone());
    let owned_union = union_discard_union_producer(
        "adapter_owned_union_source",
        &flat_members,
        HirExpr::UnionInject(
            Box::new(union_discard_call("adapter_owned_promise_source")),
            0,
            flat_members.clone(),
        ),
    );
    let borrowed_union = union_discard_function(
        "adapter_borrowed_union_source",
        vec![HirParam { name: "borrowed_source".into(), ty: promise.clone() }],
        flat.clone(),
        vec![HirStmt::Return(Some(HirExpr::UnionInject(
            Box::new(HirExpr::Var("borrowed_source".into())),
            0,
            flat_members.clone(),
        )))],
    );
    let method_unbound = union_discard_function(
        "adapter_method_unbound_union_source",
        vec![],
        flat.clone(),
        vec![HirStmt::Return(Some(union_discard_call("adapter_owned_union_source")))],
    );
    let method_explicit = union_discard_function(
        "adapter_method_explicit_union_source",
        vec![],
        flat.clone(),
        vec![HirStmt::Return(Some(union_discard_call("adapter_owned_union_source")))],
    );

    let function_ref_call = HirExpr::Call(
        Box::new(HirExpr::FunctionRef(
            "adapter_owned_union_source".into(), vec![], flat.clone(),
        )),
        vec![],
    );
    let closure_call = HirExpr::Call(
        Box::new(HirExpr::Lambda(
            vec![], vec![], flat.clone(),
            Box::new(union_discard_call("adapter_owned_union_source")),
        )),
        vec![],
    );
    let method_ref_call = HirExpr::FunctionCallWithThis(
        Box::new(HirExpr::MethodRef(
            "adapter_method_unbound_union_source".into(),
            "adapter_method_explicit_union_source".into(),
            vec![],
            flat.clone(),
            true,
            None,
        )),
        Box::new(HirExpr::Lit(HirLit::Undefined)),
        vec![],
        vec![],
        flat.clone(),
    );
    let non_arrow_call = HirExpr::FunctionCallWithThis(
        Box::new(HirExpr::NonArrowFunction(Box::new(HirExpr::Lambda(
            vec![],
            vec![HirParam { name: "__thaw_this".into(), ty: HirType::Undefined }],
            flat.clone(),
            Box::new(union_discard_call("adapter_owned_union_source")),
        )))),
        Box::new(HirExpr::Lit(HirLit::Undefined)),
        vec![],
        vec![],
        flat.clone(),
    );
    let void_lambda = HirExpr::Call(
        Box::new(HirExpr::Lambda(
            vec![], vec![], HirType::Void,
            Box::new(union_discard_call("adapter_owned_union_source")),
        )),
        vec![],
    );

    let functions = vec![
        union_discard_promise_producer("adapter_owned_promise_source", HirType::F64),
        owned_union,
        borrowed_union,
        method_unbound,
        method_explicit,
        union_discard_expression_probe(
            "discard_via_named_union_call", vec![],
            union_discard_call("adapter_owned_union_source"),
        ),
        union_discard_expression_probe(
            "discard_via_function_ref_adapter", vec![], function_ref_call,
        ),
        union_discard_expression_probe(
            "discard_via_real_closure_call", vec![], closure_call,
        ),
        union_discard_expression_probe(
            "discard_via_method_ref_this_adapter", vec![], method_ref_call,
        ),
        union_discard_expression_probe(
            "discard_via_nonarrow_this_entry", vec![], non_arrow_call,
        ),
        union_discard_expression_probe(
            "compile_real_void_expression_lambda", vec![], void_lambda,
        ),
        union_discard_expression_probe(
            "discard_borrowed_native_union_return",
            vec![HirParam { name: "borrowed_source".into(), ty: promise.clone() }],
            HirExpr::Call(
                Box::new(HirExpr::Var("adapter_borrowed_union_source".into())),
                vec![HirExpr::Var("borrowed_source".into())],
            ),
        ),
    ];
    let ir = compile_union_discard_program(
        &HirProgram { functions, ..HirProgram::default() },
        "discarded_native_union_adapters",
    );

    for (consumer_name, producer) in [
        ("discard_via_named_union_call", Some("adapter_owned_union_source")),
        ("discard_via_function_ref_adapter", None),
        ("discard_via_real_closure_call", None),
        ("discard_via_method_ref_this_adapter", None),
        ("discard_via_nonarrow_this_entry", None),
        ("discard_borrowed_native_union_return", Some("adapter_borrowed_union_source")),
    ] {
        let body = union_discard_function_body(&ir, consumer_name);
        assert_union_result_payload_reaches_destroy(&body, producer, &[0]);
        assert_pending_exception_continuation_precedes_union_discard(&body);
        assert!(!body.contains("call i8 @thaw_promise_retain("),
            "consumer must not reacquire an already-owned adapter result:\n{body}");
    }

    // Select the actual synchronous void dropper by its distinctive named
    // producer call, not by lambda emission order (PromiseNew emits a void
    // executor lambda earlier in the same module).
    let droppers = union_discard_all_function_bodies(&ir).into_iter()
        .filter(|body| body.lines().next().is_some_and(|line|
            line.starts_with("define internal void @__thaw_lambda_"))
            && body.contains("@adapter_owned_union_source("))
        .collect::<Vec<_>>();
    assert_eq!(droppers.len(), 1, "unique void expression lambda selected by its result producer:\n{ir}");
    assert_union_result_payload_reaches_destroy(&droppers[0], Some("adapter_owned_union_source"), &[0]);

    // Each forwarding shim must return its target's exact union SSA value and
    // must neither retain nor destroy. The consumer immediately above owns the
    // single discard of that transferred output token.
    let bodies = union_discard_all_function_bodies(&ir);
    let function_ref = bodies.iter().find(|body| {
        body.lines().next().is_some_and(|line| line.contains("@__thaw_function_ref_"))
            && body.contains("@adapter_owned_union_source(")
    }).expect("real function-ref forwarding adapter emitted");
    assert_adapter_returns_call_result(function_ref, "adapter_owned_union_source");

    let ignored_this = bodies.iter().find(|body| {
        body.lines().next().is_some_and(|line|
            line.contains("@__thaw_function_ref_") && line.contains("__thaw_this_adapter"))
    }).expect("ignored-this function-ref adapter emitted");
    let ignored_target = ignored_this.lines()
        .find(|line| line.contains(" = call { i8, i64 } @__thaw_function_ref_"))
        .and_then(|line| line.split(" = call").nth(1))
        .and_then(|tail| tail.split('@').nth(1))
        .and_then(|tail| tail.split('(').next())
        .expect("ignored-this adapter calls the ordinary forwarding adapter");
    assert_adapter_returns_call_result(ignored_this, ignored_target);

    let method_adapters = bodies.iter().filter(|body| {
        body.lines().next().is_some_and(|line| line.contains("@__thaw_method_ref_"))
    }).collect::<Vec<_>>();
    assert_eq!(method_adapters.len(), 2, "ordinary and explicit-this method-ref adapters both exist");
    assert_adapter_returns_call_result(
        method_adapters.iter().find(|body| body.contains("@adapter_method_unbound_union_source("))
            .expect("ordinary method-ref adapter targets unbound entry"),
        "adapter_method_unbound_union_source",
    );
    assert_adapter_returns_call_result(
        method_adapters.iter().find(|body| body.contains("@adapter_method_explicit_union_source("))
            .expect("this-aware method-ref adapter targets explicit entry"),
        "adapter_method_explicit_union_source",
    );

    let nonarrow_target = bodies.iter().find(|body| {
        let Some(signature) = body.lines().next() else { return false; };
        if !signature.starts_with("define internal { i8, i64 } @__thaw_lambda_")
            || !body.contains("@adapter_owned_union_source(")
        {
            return false;
        }
        let name = signature.split('@').nth(1).unwrap().split('(').next().unwrap();
        ["__ordinary", "__thaw_this_adapter"].iter().all(|suffix| {
            let expected = format!("@{name}{suffix}(");
            bodies.iter().any(|candidate|
                candidate.lines().next().is_some_and(|line| line.contains(&expected)))
        })
    }).expect("constructed non-arrow closure target emitted with both real entries");
    let target_name = nonarrow_target.lines().next().unwrap()
        .split('@').nth(1).unwrap().split('(').next().unwrap();
    for suffix in ["__ordinary", "__thaw_this_adapter"] {
        let adapter_name = format!("{target_name}{suffix}");
        let adapter = bodies.iter().find(|body|
            body.lines().next().is_some_and(|line| line.contains(&format!("@{adapter_name}("))))
            .unwrap_or_else(|| panic!("missing constructed non-arrow entry {adapter_name}"));
        assert_adapter_returns_call_result(adapter, target_name);
    }

    let borrowed_producer = union_discard_function_body(&ir, "adapter_borrowed_union_source");
    let retain_calls = borrowed_producer.lines()
        .filter(|line| line.contains("call i8 @thaw_promise_retain("))
        .collect::<Vec<_>>();
    assert_eq!(retain_calls.len(), 1,
        "borrowed native return acquires exactly one output token:\n{borrowed_producer}");
    assert!(retain_calls[0].contains("borrowed_source"),
        "the producer's retain operand is derived from its borrowed input:\n{borrowed_producer}");
    let borrowed_consumer = union_discard_function_body(&ir, "discard_borrowed_native_union_return");
    assert_union_call_payload_reaches_destroy(
        &borrowed_consumer, "adapter_borrowed_union_source", &[0],
    );
    assert_borrowed_promise_cleanup_flag_is_false_initialized(&borrowed_consumer, "borrowed_source");

    // Native source liveness through an actual borrowed-return producer:
    // discarding its owned union output must leave the original owner usable.
    let source = r#"
        function borrowedUnion(value: Promise<number>): Promise<number> | number {
            return value;
        }
        function main(): void {
            const owner: Promise<number> = Promise.resolve(42);
            borrowedUnion(owner);
            owner.then((value: number): void => { console.log(value); });
        }
    "#;
    assert_eq!(compile_and_run(source, "borrowed_native_union_return_owner_live"), "42\n");
}
