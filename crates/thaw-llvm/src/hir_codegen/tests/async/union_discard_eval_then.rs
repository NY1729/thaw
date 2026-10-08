// Exact EvalThen discard, result forwarding, pending-exception CFG, and
// codegen-context restoration controls. These tests are authored for the
// later immutable supported lane; the malformed restoration fixture is
// codegen-only and must never be invoked as native code.

static UDEVAL_EXCEPTION_EFFECT_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static UDEVAL_EXCEPTION_PROBE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

extern "C" fn udeval_mapped_exception_effect() -> f64 {
    UDEVAL_EXCEPTION_EFFECT_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    9.0
}

extern "C" fn udeval_non_dereferencing_destroy(_token: *mut std::ffi::c_void) {}

fn udeval_flat_union_type() -> HirType {
    HirType::Union(vec![
        HirType::Promise(Box::new(HirType::F64)),
        HirType::F64,
    ])
}

fn udeval_three_union_type() -> HirType {
    HirType::Union(vec![
        HirType::Promise(Box::new(HirType::F64)),
        HirType::Str,
        HirType::Promise(Box::new(HirType::I64)),
    ])
}

fn udeval_call(name: &str) -> HirExpr {
    HirExpr::Call(Box::new(HirExpr::Var(name.into())), Vec::new())
}

fn udeval_eval_then(first: HirExpr, second: HirExpr) -> HirExpr {
    HirExpr::EvalThen(Box::new(first), Box::new(second))
}

fn udeval_function_body<'a>(ir: &'a str, function: &str) -> String {
    let signature = format!("@{function}(");
    let body = ir
        .lines()
        .skip_while(|line| !(line.starts_with("define ") && line.contains(&signature)))
        .take_while(|line| *line != "}")
        .collect::<Vec<_>>();
    assert!(!body.is_empty(), "missing function {function} in IR:\n{ir}");
    body.join("\n")
}

struct UdevalBlock {
    label: String,
    text: String,
}

fn udeval_blocks(function_body: &str) -> Vec<UdevalBlock> {
    let mut blocks = Vec::<UdevalBlock>::new();
    let mut current_label = String::new();
    let mut current_text = String::new();
    for line in function_body.lines() {
        let code = line.split(';').next().unwrap_or(line).trim();
        if code.ends_with(':') {
            if !current_label.is_empty() {
                blocks.push(UdevalBlock {
                    label: std::mem::take(&mut current_label),
                    text: std::mem::take(&mut current_text),
                });
            }
            current_label = code.trim_end_matches(':').trim_matches('"').to_string();
        } else if !current_label.is_empty() {
            current_text.push_str(line);
            current_text.push('\n');
        }
    }
    if !current_label.is_empty() {
        blocks.push(UdevalBlock { label: current_label, text: current_text });
    }
    blocks
}

fn udeval_successors(block: &UdevalBlock) -> Vec<String> {
    let terminator = block.text.lines().map(str::trim).filter(|line|
        line.starts_with("br ") || line.starts_with("ret ") || line.starts_with("unreachable")
    ).last().unwrap_or("");
    terminator.split("label %").skip(1).map(|tail|
        tail.split(|ch: char| ch == ',' || ch == ']' || ch.is_whitespace())
            .next().unwrap_or("").trim_matches('"').to_string()
    ).filter(|label| !label.is_empty()).collect()
}

fn udeval_block<'a>(blocks: &'a [UdevalBlock], label: &str) -> &'a UdevalBlock {
    blocks.iter().find(|block| block.label == label)
        .unwrap_or_else(|| panic!("missing LLVM block {label}; blocks: {:?}",
            blocks.iter().map(|block| &block.label).collect::<Vec<_>>()))
}

fn udeval_reaches(blocks: &[UdevalBlock], from: &str, to: &str) -> bool {
    let mut pending = vec![from.to_string()];
    let mut seen = std::collections::HashSet::new();
    while let Some(label) = pending.pop() {
        if label == to { return true; }
        if !seen.insert(label.clone()) { continue; }
        if let Some(block) = blocks.iter().find(|block| block.label == label) {
            pending.extend(udeval_successors(block));
        }
    }
    false
}

fn udeval_reaches_avoiding(blocks: &[UdevalBlock], from: &str, to: &str, avoid: &str) -> bool {
    if from == avoid { return false; }
    let mut pending = vec![from.to_string()];
    let mut seen = std::collections::HashSet::new();
    while let Some(label) = pending.pop() {
        if label == to { return true; }
        if label == avoid || !seen.insert(label.clone()) { continue; }
        if let Some(block) = blocks.iter().find(|block| block.label == label) {
            pending.extend(udeval_successors(block));
        }
    }
    false
}

fn udeval_dominates(blocks: &[UdevalBlock], dominator: &str, target: &str) -> bool {
    // Dominance is reflexive: the next evaluation may sit in the merge block itself.
    udeval_reaches(blocks, "entry", target)
        && (dominator == "entry"
            || dominator == target
            || !udeval_reaches_avoiding(blocks, "entry", target, dominator))
}

fn udeval_call_block<'a>(blocks: &'a [UdevalBlock], callee: &str) -> &'a UdevalBlock {
    let call_lines = blocks.iter().flat_map(|block| block.text.lines()).filter(|line|
        line.contains("= call ") && line.contains(&format!("@{callee}("))
    ).collect::<Vec<_>>();
    assert_eq!(call_lines.len(), 1,
        "expected exactly one SSA-producing call to {callee}; blocks: {:?}",
        blocks.iter().map(|block| (&block.label, &block.text)).collect::<Vec<_>>());
    let matches = blocks.iter().filter(|block| block.text.lines().any(|line|
        line.contains("= call ") && line.contains(&format!("@{callee}("))))
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1,
        "expected exactly one SSA-producing call to {callee}; blocks: {:?}",
        blocks.iter().map(|block| (&block.label, &block.text)).collect::<Vec<_>>());
    matches[0]
}

fn udeval_call_result(function_body: &str, callee: &str) -> String {
    let matches = function_body.lines().filter(|line|
        line.contains("= call ") && line.contains(&format!("@{callee}("))
    ).collect::<Vec<_>>();
    assert_eq!(matches.len(), 1,
        "expected exactly one SSA-producing call to {callee}:\n{function_body}");
    matches[0].split_once('=').unwrap().0.trim().to_string()
}

fn udeval_branch_condition(line: &str) -> Option<String> {
    line.trim_start().strip_prefix("br i1 ")
        .map(|tail| tail.split(',').next().unwrap().trim().to_string())
}

fn udeval_call_edges(blocks: &[UdevalBlock], call_block: &UdevalBlock) -> (String, String) {
    let successors = udeval_successors(call_block);
    assert_eq!(successors.len(), 2,
        "native call must branch on the pending-exception predicate:\n{}", call_block.text);
    let branch_line = call_block.text.lines().find(|line|
        line.trim_start().starts_with("br i1 ")
    ).expect("native call must end in a conditional exception branch");
    let condition = udeval_branch_condition(branch_line).unwrap();
    let pending_loads = call_block.text.lines().filter(|line|
        line.split_once("= load ptr, ptr ").is_some_and(|(_, operand)|
            operand.split(',').next().unwrap().trim() == "@__thaw_pending_exception"
        )
    ).collect::<Vec<_>>();
    assert_eq!(pending_loads.len(), 1,
        "call CFG must contain one exact pending-exception pointer load:\n{}", call_block.text);
    let load_line = pending_loads[0];
    let pending_value = udeval_ssa_result(load_line);
    let comparison = call_block.text.lines().filter(|line|
        line.contains("= icmp ne ptr")
            && line.contains(&format!(" {pending_value}, null"))
    ).collect::<Vec<_>>();
    assert_eq!(comparison.len(), 1,
        "pending predicate must be exactly `icmp ne ptr <loaded pending>, null`:\n{}", call_block.text);
    assert_eq!(udeval_ssa_result(comparison[0]), condition,
        "conditional branch must consume the exact pending-pointer comparison");

    // LLVM conditional branch successor order is true, then false. Production
    // branches true to exception cleanup and false to the normal continuation.
    let true_successor = &successors[0];
    let false_successor = &successors[1];
    assert!(true_successor.starts_with("propagate_exception")
            || true_successor.starts_with("cleanup_before_catch"),
        "true pending predicate edge must be the exception successor, got {true_successor}");
    assert!(false_successor.starts_with("call_ok"),
        "false pending predicate edge must be the normal call continuation, got {false_successor}");
    assert!(!udeval_block(blocks, true_successor).text.is_empty());
    assert!(!udeval_block(blocks, false_successor).text.is_empty());
    (false_successor.clone(), true_successor.clone())
}

fn udeval_assert_pending_edges_skip_later(
    blocks: &[UdevalBlock],
    first_callee: &str,
    later_callee: &str,
) {
    let first = udeval_call_block(blocks, first_callee);
    let (normal, exceptional) = udeval_call_edges(blocks, first);
    let later = udeval_call_block(blocks, later_callee);
    assert!(udeval_reaches(blocks, &normal, &later.label),
        "normal successor of {first_callee} must reach later call {later_callee}");
    assert!(!udeval_reaches(blocks, &exceptional, &later.label),
        "pending-exception successor of {first_callee} must bypass {later_callee}");
}

fn udeval_assert_result_destroy_only_on_normal_edge(
    blocks: &[UdevalBlock],
    callee: &str,
    trace: &UdevalUnionTrace,
) {
    let call = udeval_call_block(blocks, callee);
    let (normal, exceptional) = udeval_call_edges(blocks, call);
    for destroy in &trace.destroy_blocks {
        assert!(udeval_reaches(blocks, &normal, destroy),
            "normal result continuation must reach its selected discard");
        assert!(!udeval_reaches(blocks, &exceptional, destroy),
            "pending-exception successor must bypass selected result discard");
    }
}

struct UdevalUnionTrace {
    merge: String,
    destroy_blocks: Vec<String>,
}

fn udeval_ssa_result(line: &str) -> String {
    line.split_once('=').expect("SSA definition").0.trim().to_string()
}

fn udeval_assert_union_destroy_identity(
    function_body: &str,
    blocks: &[UdevalBlock],
    aggregate: &str,
    selected_tags: &[u8],
) -> UdevalUnionTrace {
    let aggregate_operand = format!(" {aggregate},");
    let tag_lines = function_body.lines().filter(|line|
        line.contains("= extractvalue") && line.contains(&aggregate_operand) && line.ends_with(", 0")
    ).collect::<Vec<_>>();
    assert_eq!(tag_lines.len(), 1, "expected one exact tag extraction from {aggregate}:\n{function_body}");
    let tag_value = udeval_ssa_result(tag_lines[0]);
    let mut destroy_blocks = Vec::new();
    let mut previous_false = None;
    let mut merge = None;

    for selected_tag in selected_tags {
        let compare_lines = function_body.lines().filter(|line|
            line.contains("= icmp eq i8")
                && line.contains(&format!(" {tag_value}, {selected_tag}"))
                && line.trim_end().ends_with(&format!(", {selected_tag}"))
        ).collect::<Vec<_>>();
        assert_eq!(compare_lines.len(), 1,
            "expected one exact union-tag comparison for {aggregate} tag {selected_tag}:\n{function_body}");
        let condition = udeval_ssa_result(compare_lines[0]);
        let branches = blocks.iter().filter(|block|
            block.text.lines().any(|line|
                udeval_branch_condition(line).as_deref() == Some(condition.as_str())
            )
        ).collect::<Vec<_>>();
        assert_eq!(branches.len(), 1,
            "tag condition {condition} must control exactly one branch:\n{function_body}");
        let branch = branches[0];
        if let Some(expected_test) = &previous_false {
            assert_eq!(&branch.label, expected_test,
                "the previous rejected-tag edge must reach this next tag test");
        }
        let successors = udeval_successors(branch);
        assert_eq!(successors.len(), 2, "tag test must have selected and fallthrough edges");
        let selected_block = successors[0].clone();
        let rejected = successors[1].clone();
        assert!(selected_block.starts_with("discard_union_promise"),
            "true tag-match edge must enter the selected Promise block, got {selected_block}");
        assert!(rejected.starts_with("discard_union_next"),
            "false tag edge must enter the next-tag/plain continuation, got {rejected}");
        let selected_text = &udeval_block(blocks, &selected_block).text;
        let payload_lines = selected_text.lines().filter(|line|
            line.contains("= extractvalue")
                && line.contains(&aggregate_operand)
                && line.ends_with(", 1")
        ).collect::<Vec<_>>();
        assert_eq!(payload_lines.len(), 1,
            "each selected tag block must locally extract its own payload SSA value:\n{selected_text}");
        let payload_value = udeval_ssa_result(payload_lines[0]);
        let int_to_ptr_lines = selected_text.lines().filter(|line|
            line.contains("= inttoptr i64")
                && line.contains(&format!(" {payload_value} to ptr"))
        ).collect::<Vec<_>>();
        assert_eq!(int_to_ptr_lines.len(), 1,
            "selected tag must decode its local payload exactly once:\n{selected_text}");
        let int_to_ptr_line = int_to_ptr_lines[0];
        let decoded_pointer = udeval_ssa_result(int_to_ptr_line);
        let selected_releases = selected_text.lines().filter(|line|
            line.contains("@thaw_promise_destroy(")
                && line.contains(&format!("ptr {decoded_pointer})"))
        ).collect::<Vec<_>>();
        assert_eq!(selected_releases.len(), 1,
            "selected tag block must destroy its exact decoded token exactly once:\n{selected_text}");
        let exact_releases_in_function = function_body.lines().filter(|line|
            line.contains("@thaw_promise_destroy(")
                && line.contains(&format!("ptr {decoded_pointer})"))
        ).count();
        assert_eq!(exact_releases_in_function, 1,
            "this selected SSA token must have exactly one release in its consumer function");
        let selected_successors = udeval_successors(udeval_block(blocks, &selected_block));
        assert_eq!(selected_successors.len(), 1,
            "selected destroy path must continue to the live normal merge:\n{selected_text}");
        let selected_merge = selected_successors[0].clone();
        assert!(selected_merge.starts_with("discarded_union_end"),
            "selected release must join the actual normal union merge, got {selected_merge}");
        if let Some(previous) = &merge {
            assert_eq!(previous, &selected_merge,
                "each Promise-tag release must reach the same live union merge");
        } else {
            merge = Some(selected_merge);
        }
        destroy_blocks.push(selected_block);
        let rejected_text = &udeval_block(blocks, &rejected).text;
        assert!(!rejected_text.contains("= extractvalue")
                && !rejected_text.contains("= inttoptr i64")
                && !rejected_text.contains("@thaw_promise_destroy("),
            "false tag edge must not extract/decode/destroy an inactive payload:\n{rejected_text}");
        previous_false = Some(rejected);
    }

    let merge = merge.expect("at least one Promise tag");
    let final_false = previous_false.expect("final non-Promise fallthrough");
    let final_false_block = udeval_block(blocks, &final_false);
    assert!(!final_false_block.text.contains("= extractvalue")
            && !final_false_block.text.contains("= inttoptr i64")
            && !final_false_block.text.contains("@thaw_promise_destroy("),
        "plain/non-Promise fallthrough must not extract/decode/destroy the payload:\n{}", final_false_block.text);
    assert_eq!(udeval_successors(final_false_block), vec![merge.clone()],
        "the final rejected tag must reach the same live continuation");
    UdevalUnionTrace {
        merge,
        destroy_blocks,
    }
}

fn udeval_assert_exact_promise_destroy_identity(
    function_body: &str,
    promise_call: &str,
) -> String {
    let promise = udeval_call_result(function_body, promise_call);
    let packed_lines = function_body.lines().filter(|line|
        line.contains("= ptrtoint ptr") && line.contains(&format!(" {promise} to i64"))
    ).collect::<Vec<_>>();
    assert_eq!(packed_lines.len(), 1,
        "second Promise SSA token {promise} must be packed exactly once:\n{function_body}");
    let packed_word_line = packed_lines[0];
    let packed_word = udeval_ssa_result(packed_word_line);
    let tagged_union_lines = function_body.lines().filter(|line|
        line.contains("= insertvalue") && line.contains(&format!("i64 {packed_word},"))
            && line.trim_end().ends_with(", 1")
    ).collect::<Vec<_>>();
    assert_eq!(tagged_union_lines.len(), 1,
        "wrapper must insert the same Promise word exactly once:\n{function_body}");
    let tagged_union_line = tagged_union_lines[0];
    let tagged_union = udeval_ssa_result(tagged_union_line);
    let tagged_union_operand = format!(" {tagged_union},");
    let tag_lines = function_body.lines().filter(|line|
        line.contains("= extractvalue") && line.contains(&tagged_union_operand) && line.ends_with(", 0")
    ).collect::<Vec<_>>();
    assert_eq!(tag_lines.len(), 1, "wrapper must extract its tag exactly once");
    let tag = udeval_ssa_result(tag_lines[0]);
    let compare_lines = function_body.lines().filter(|line|
        line.contains("= icmp eq i8")
            && line.contains(&format!(" {tag}, 0"))
            && line.trim_end().ends_with(", 0")
    ).collect::<Vec<_>>();
    assert_eq!(compare_lines.len(), 1, "wrapper must compare its exact tag to zero once");
    let condition = udeval_ssa_result(compare_lines[0]);
    let blocks = udeval_blocks(function_body);
    let branch_blocks = blocks.iter().filter(|block| block.text.lines().any(|line|
        udeval_branch_condition(line).as_deref() == Some(condition.as_str())
    )).collect::<Vec<_>>();
    assert_eq!(branch_blocks.len(), 1, "wrapper tag predicate must control one branch");
    let branch_block = branch_blocks[0];
    let successors = udeval_successors(branch_block);
    assert_eq!(successors.len(), 2);
    let selected_block = udeval_block(&blocks, &successors[0]);
    let rejected_block = udeval_block(&blocks, &successors[1]);
    assert!(successors[0].starts_with("discard_union_promise"),
        "true tag match must select the Promise block");
    assert!(successors[1].starts_with("discard_union_next"),
        "false tag match must continue to the plain path");
    let payload_lines = selected_block.text.lines().filter(|line|
        line.contains("= extractvalue") && line.contains(&tagged_union_operand) && line.ends_with(", 1")
    ).collect::<Vec<_>>();
    assert_eq!(payload_lines.len(), 1,
        "selected wrapper tag must locally extract the exact payload once:\n{}", selected_block.text);
    let payload = udeval_ssa_result(payload_lines[0]);
    let decoded_lines = selected_block.text.lines().filter(|line|
        line.contains("= inttoptr i64") && line.contains(&format!(" {payload} to ptr"))
    ).collect::<Vec<_>>();
    assert_eq!(decoded_lines.len(), 1,
        "selected wrapper payload must decode exactly once:\n{}", selected_block.text);
    let decoded = udeval_ssa_result(decoded_lines[0]);
    let selected_releases = selected_block.text.lines().filter(|line|
        line.contains("@thaw_promise_destroy(") && line.contains(&format!("ptr {decoded})"))
    ).collect::<Vec<_>>();
    assert_eq!(selected_releases.len(), 1,
        "selected wrapper token must be released once:\n{}", selected_block.text);
    assert_eq!(function_body.lines().filter(|line|
        line.contains("@thaw_promise_destroy(") && line.contains(&format!("ptr {decoded})"))
    ).count(), 1, "the wrapper's exact decoded pointer must have one release site");
    let merge = udeval_successors(selected_block);
    assert_eq!(merge.len(), 1);
    assert!(merge[0].starts_with("discarded_union_end"),
        "selected wrapper destruction must reach its normal merge");
    assert!(!rejected_block.text.contains("= extractvalue")
            && !rejected_block.text.contains("= inttoptr i64")
            && !rejected_block.text.contains("@thaw_promise_destroy("),
        "false wrapper-tag edge must not extract/decode/destroy its payload");
    assert_eq!(udeval_successors(rejected_block), vec![merge[0].clone()]);
    decoded
}

fn udeval_compile_case(
    function_name: &str,
    return_type: HirType,
    params: &[HirParam],
    statement: HirStmt,
    native_returns: &[(&str, HirType)],
) -> String {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, function_name);
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    for (name, return_type) in native_returns {
        compiler.function_return_types.insert((*name).into(), return_type.clone());
        let native_type = compiler.basic_type(return_type).unwrap();
        compiler.module.add_function(
            name,
            native_type.fn_type(&[], false),
            None,
        );
    }

    let llvm_params = params.iter().map(|param|
        compiler.basic_type(&param.ty).map(BasicMetadataTypeEnum::from)
    ).collect::<Result<Vec<_>, _>>().unwrap();
    let function_type = if return_type == HirType::Void {
        context.void_type().fn_type(&llvm_params, false)
    } else {
        compiler.basic_type(&return_type).unwrap().fn_type(&llvm_params, false)
    };
    let function = compiler.module.add_function(function_name, function_type, Some(Linkage::Internal));
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    for (index, param) in params.iter().enumerate() {
        let ty = compiler.basic_type(&param.ty).unwrap();
        let cell = compiler.builder.build_alloca(ty, &format!("{}_cell", param.name)).unwrap();
        compiler.builder.build_store(cell, function.get_nth_param(index as u32).unwrap()).unwrap();
        compiler.variables.insert(param.name.clone(), (cell, ty));
        compiler.variable_hir_types.insert(param.name.clone(), param.ty.clone());
    }

    let terminated = compiler.compile_stmt(&statement).unwrap();
    if return_type == HirType::Void && !terminated {
        compiler.builder.build_return(None).unwrap();
    }
    compiler.module.verify().unwrap();
    compiler.print_to_string()
}

fn udeval_compile_exception_case() -> String {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "udeval_exception_effect_edges");
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    let flat = udeval_flat_union_type();
    for (name, hir_type) in [
        ("udeval_throwing_first_union", flat.clone()),
        ("udeval_after_first_effect", HirType::F64),
    ] {
        compiler.function_return_types.insert(name.into(), hir_type.clone());
        let native_type = compiler.basic_type(&hir_type).unwrap();
        compiler.module.add_function(name, native_type.fn_type(&[], false), None);
    }
    let function = compiler.module.add_function(
        "udeval_exception_effects",
        context.void_type().fn_type(&[], false),
        Some(Linkage::Internal),
    );
    let entry = context.append_basic_block(function, "entry");
    let caught = context.append_basic_block(function, "udeval_caught_exception");
    compiler.builder.position_at_end(entry);
    compiler.push_catch_target(caught);
    let statement = HirStmt::Expr(udeval_eval_then(
        udeval_call("udeval_throwing_first_union"),
        udeval_call("udeval_after_first_effect"),
    ));
    let terminated = compiler.compile_stmt(&statement).unwrap();
    assert!(!terminated);
    compiler.builder.build_return(None).unwrap();
    compiler.pop_catch_target();
    compiler.builder.position_at_end(caught);
    compiler.builder.build_return(None).unwrap();
    compiler.module.verify().unwrap();
    compiler.print_to_string()
}

fn udeval_compile_direct_owned_second_value() -> String {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "udeval_direct_owned_second_value");
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    let first_type = udeval_flat_union_type();
    let second_type = udeval_three_union_type();
    for (name, return_type) in [
        ("udeval_two_first_flat", first_type),
        ("udeval_two_second_three", second_type.clone()),
    ] {
        compiler.function_return_types.insert(name.into(), return_type.clone());
        let llvm_type = compiler.basic_type(&return_type).unwrap();
        compiler.module.add_function(name, llvm_type.fn_type(&[], false), None);
    }
    let function_type = compiler.basic_type(&second_type).unwrap().fn_type(&[], false);
    let function = compiler.module.add_function(
        "udeval_direct_expr_value_returns_second",
        function_type,
        Some(Linkage::Internal),
    );
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let expression = udeval_eval_then(
        udeval_call("udeval_two_first_flat"),
        udeval_call("udeval_two_second_three"),
    );
    // Deliberately call the actual value-expression compiler and emit the
    // LLVM Return directly, bypassing production Return(EvalThen) ownership
    // normalization and preserving the P1 HOLD boundary.
    let returned = compiler.compile_expr(&expression).unwrap();
    compiler.builder.build_return(Some(&returned)).unwrap();
    compiler.module.verify().unwrap();
    compiler.print_to_string()
}

fn udeval_run_mapped_exception_effect_probe() {
    let _guard = UDEVAL_EXCEPTION_PROBE_LOCK.lock().unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "udeval_mapped_exception_effect_probe");
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    let flat = udeval_flat_union_type();
    let native_thrower = compiler.module.add_function(
        "udeval_mapped_throwing_union",
        compiler.basic_type(&flat).unwrap().fn_type(&[], false),
        Some(Linkage::Internal),
    );
    compiler.function_return_types.insert("udeval_mapped_throwing_union".into(), flat.clone());
    let native_entry = context.append_basic_block(native_thrower, "entry");
    compiler.builder.position_at_end(native_entry);
    assert!(compiler.compile_stmt(&HirStmt::Throw(
        HirExpr::Lit(HirLit::Str("mapped EvalThen first threw".into())),
    )).unwrap());

    let effect_type = compiler.basic_type(&HirType::F64).unwrap().fn_type(&[], false);
    compiler.module.add_function("udeval_mapped_after_throw", effect_type, None);
    compiler.function_return_types.insert("udeval_mapped_after_throw".into(), HirType::F64);
    let probe = compiler.module.add_function(
        "udeval_mapped_exception_effects",
        context.void_type().fn_type(&[], false),
        Some(Linkage::Internal),
    );
    let probe_entry = context.append_basic_block(probe, "entry");
    compiler.builder.position_at_end(probe_entry);
    assert!(!compiler.compile_stmt(&HirStmt::Expr(udeval_eval_then(
        udeval_call("udeval_mapped_throwing_union"),
        udeval_call("udeval_mapped_after_throw"),
    ))).unwrap());
    compiler.builder.build_return(None).unwrap();

    // This test-only reset writes every member of the pending-exception tuple,
    // so the fixture can be repeated without carrying stale native/text/tag
    // state between invocations. It has no host or runtime dependencies.
    let reset = compiler.module.add_function(
        "udeval_reset_pending_exception_tuple",
        context.void_type().fn_type(&[], false),
        Some(Linkage::Internal),
    );
    let reset_entry = context.append_basic_block(reset, "entry");
    compiler.builder.position_at_end(reset_entry);
    for symbol in [
        "__thaw_pending_exception",
        "__thaw_pending_rejection",
        "__thaw_pending_exception_native_text",
        "__thaw_pending_exception_object",
        "__thaw_pending_exception_native",
        "__thaw_pending_exception_aggregate_errors",
    ] {
        let global = compiler.module.get_global(symbol).expect("exception tuple pointer global");
        compiler.builder.build_store(
            global.as_pointer_value(),
            context.ptr_type(AddressSpace::default()).const_null(),
        ).unwrap();
    }
    let scalar_zeros: [(&str, inkwell::values::BasicValueEnum<'_>); 4] = [
        ("__thaw_pending_exception_value_tag", context.i64_type().const_zero().into()),
        ("__thaw_pending_exception_f64", context.f64_type().const_zero().into()),
        ("__thaw_pending_exception_i64", context.i64_type().const_zero().into()),
        ("__thaw_pending_exception_bool", context.bool_type().const_zero().into()),
    ];
    for (symbol, zero) in scalar_zeros {
        let global = compiler.module.get_global(symbol).expect("exception tuple scalar global");
        compiler.builder.build_store(global.as_pointer_value(), zero).unwrap();
    }
    compiler.builder.build_return(None).unwrap();

    // Close the entire executable external-call dependency set before JIT:
    // the effect counter and Promise destroy spy are both ABI-correct, the
    // latter ignores its inert pointer and never dereferences a Promise token.
    let ir = compiler.print_to_string();
    let mut external_calls = std::collections::HashSet::<String>::new();
    for line in ir.lines().filter(|line| line.contains("call ") && line.contains('@')) {
        let symbol = line.split_once('@').unwrap().1.split('(').next().unwrap()
            .trim_matches('"').to_string();
        let function = compiler.module.get_function(&symbol)
            .unwrap_or_else(|| panic!("call target missing from module: {symbol}"));
        if function.get_basic_blocks().is_empty() {
            external_calls.insert(symbol);
        }
    }
    let expected_external_calls: std::collections::HashSet<String> = [
        "thaw_promise_destroy".to_string(),
        "udeval_mapped_after_throw".to_string(),
    ].into_iter().collect();
    assert_eq!(external_calls, expected_external_calls,
        "every generated external call in the isolated module must have an explicit mapped stub:\n{ir}");
    for symbol in [
        "__thaw_pending_exception",
        "__thaw_pending_rejection",
        "__thaw_pending_exception_native_text",
        "__thaw_pending_exception_object",
        "__thaw_pending_exception_native",
        "__thaw_pending_exception_aggregate_errors",
        "__thaw_pending_exception_value_tag",
        "__thaw_pending_exception_f64",
        "__thaw_pending_exception_i64",
        "__thaw_pending_exception_bool",
    ] {
        assert!(udeval_function_body(&ir, "udeval_reset_pending_exception_tuple").contains(symbol),
            "test-only reset must clear every pending-exception tuple field: {symbol}");
    }

    compiler.module.verify().unwrap();
    let engine = compiler.module.create_jit_execution_engine(OptimizationLevel::None).unwrap();
    engine.add_global_mapping(
        &compiler.module.get_function("thaw_promise_destroy").unwrap(),
        udeval_non_dereferencing_destroy as usize,
    );
    engine.add_global_mapping(
        &compiler.module.get_function("udeval_mapped_after_throw").unwrap(),
        udeval_mapped_exception_effect as usize,
    );
    type VoidProbe = unsafe extern "C" fn();
    unsafe {
        let reset = engine.get_function::<VoidProbe>("udeval_reset_pending_exception_tuple").unwrap();
        let probe = engine.get_function::<VoidProbe>("udeval_mapped_exception_effects").unwrap();
        for _ in 0..2 {
            UDEVAL_EXCEPTION_EFFECT_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
            reset.call();
            probe.call();
            assert_eq!(UDEVAL_EXCEPTION_EFFECT_CALLS.load(std::sync::atomic::Ordering::SeqCst), 0,
                "first-operand exception must bypass the mapped second effect on each isolated run");
            reset.call();
        }
    }
}

type UdevalCatchSnapshot<'ctx> = (
    inkwell::basic_block::BasicBlock<'ctx>,
    std::collections::HashSet<PointerValue<'ctx>>,
);
type UdevalLoopSnapshot<'ctx> = (
    inkwell::basic_block::BasicBlock<'ctx>,
    inkwell::basic_block::BasicBlock<'ctx>,
    std::collections::HashSet<PointerValue<'ctx>>,
);
type UdevalPromotionSnapshot<'ctx> = (
    inkwell::basic_block::BasicBlock<'ctx>,
    std::collections::HashSet<String>,
    Vec<UdevalCatchSnapshot<'ctx>>,
    std::collections::HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    Vec<UdevalLoopSnapshot<'ctx>>,
);

struct UdevalScopeSnapshot<'ctx> {
    variables: std::collections::HashMap<String, (PointerValue<'ctx>, String)>,
    catch_native_text: std::collections::HashMap<String, (
        PointerValue<'ctx>, PointerValue<'ctx>, PointerValue<'ctx>,
        PointerValue<'ctx>, PointerValue<'ctx>,
    )>,
    variable_hir_types: std::collections::HashMap<String, HirType>,
    arena_variables: std::collections::HashSet<String>,
    stack_promise_slots: std::collections::HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    for_iteration_frame_slots: std::collections::HashMap<String, PointerValue<'ctx>>,
    catch_stack: Vec<UdevalCatchSnapshot<'ctx>>,
    loop_scopes: Vec<UdevalLoopSnapshot<'ctx>>,
    loop_promotion_scopes: Vec<UdevalPromotionSnapshot<'ctx>>,
    active_async_completion: Option<PointerValue<'ctx>>,
    insertion_block: Option<inkwell::basic_block::BasicBlock<'ctx>>,
}

fn udeval_snapshot_scope<'ctx>(compiler: &HirCompiler<'ctx>) -> UdevalScopeSnapshot<'ctx> {
    UdevalScopeSnapshot {
        variables: compiler.variables.iter().map(|(name, (cell, ty))|
            (name.clone(), (*cell, ty.print_to_string().to_string()))).collect(),
        catch_native_text: compiler.catch_native_text.clone(),
        variable_hir_types: compiler.variable_hir_types.clone(),
        arena_variables: compiler.arena_variables.clone(),
        stack_promise_slots: compiler.stack_promise_slots.clone(),
        for_iteration_frame_slots: compiler.for_iteration_frame_slots.clone(),
        catch_stack: compiler.catch_stack.iter().map(|scope|
            (scope.target, scope.promise_boundary.clone())).collect(),
        loop_scopes: compiler.loop_scopes.iter().map(|scope|
            (scope.continue_target, scope.break_target, scope.promise_boundary.clone())).collect(),
        loop_promotion_scopes: compiler.loop_promotion_scopes.iter().map(|scope| (
            scope.preheader,
            scope.variables.clone(),
            scope.catches.scopes.iter().map(|catch|
                (catch.target, catch.promise_boundary.clone())).collect(),
            scope.stack_promise_slots.clone(),
            scope.outer_loops.iter().map(|loop_scope| (
                loop_scope.continue_target,
                loop_scope.break_target,
                loop_scope.promise_boundary.clone(),
            )).collect(),
        )).collect(),
        active_async_completion: compiler.active_async_completion,
        insertion_block: compiler.builder.get_insert_block(),
    }
}

fn udeval_assert_scope_restored<'ctx>(
    compiler: &HirCompiler<'ctx>,
    before: &UdevalScopeSnapshot<'ctx>,
) {
    let after = udeval_snapshot_scope(compiler);
    assert_eq!(after.variables, before.variables, "variable cells/types changed");
    assert_eq!(after.catch_native_text, before.catch_native_text, "catch text mapping changed");
    assert_eq!(after.variable_hir_types, before.variable_hir_types, "HIR variable types changed");
    assert_eq!(after.arena_variables, before.arena_variables, "arena-variable set changed");
    assert_eq!(after.stack_promise_slots, before.stack_promise_slots,
        "physical Promise slot-to-flag mapping changed");
    assert_eq!(after.for_iteration_frame_slots, before.for_iteration_frame_slots,
        "for-iteration frame slots changed");
    assert_eq!(after.catch_stack, before.catch_stack, "catch stack changed");
    assert_eq!(after.loop_scopes, before.loop_scopes, "loop stack changed");
    assert_eq!(after.loop_promotion_scopes, before.loop_promotion_scopes,
        "loop-promotion context changed");
    assert_eq!(after.active_async_completion, before.active_async_completion,
        "active async completion changed");
    assert_eq!(after.insertion_block, before.insertion_block,
        "builder insertion block was not restored to the outer block");
}

fn udeval_seed_error_compiler<'ctx>(
    compiler: &mut HirCompiler<'ctx>,
    function_name: &str,
) -> inkwell::basic_block::BasicBlock<'ctx> {
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    let function = compiler.module.add_function(
        function_name,
        compiler.context.void_type().fn_type(&[], false),
        Some(Linkage::Internal),
    );
    let entry = compiler.context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let ptr = compiler.context.ptr_type(AddressSpace::default());
    let f64_type = compiler.context.f64_type();

    let capture_cell = compiler.builder.build_alloca(f64_type, "udeval_transient_capture_stack_cell").unwrap();
    compiler.builder.build_store(capture_cell, f64_type.const_float(12.5)).unwrap();
    compiler.variables.insert("udeval_transient_capture".into(), (capture_cell, f64_type.into()));
    compiler.variable_hir_types.insert("udeval_transient_capture".into(), HirType::F64);

    let retained_cell = compiler.builder.build_alloca(f64_type, "udeval_retained_outer_cell").unwrap();
    compiler.builder.build_store(retained_cell, f64_type.const_float(6.0)).unwrap();
    compiler.variables.insert("udeval_retained_outer".into(), (retained_cell, f64_type.into()));
    compiler.variable_hir_types.insert("udeval_retained_outer".into(), HirType::F64);
    compiler.arena_variables.insert("udeval_retained_outer".into());

    let owner_slot = compiler.builder.build_alloca(ptr, "udeval_seeded_outer_promise_slot").unwrap();
    compiler.builder.build_store(owner_slot, ptr.const_null()).unwrap();
    compiler.register_stack_promise_slot(owner_slot, "udeval_seeded_outer_promise").unwrap();
    compiler.variables.insert(
        "udeval_seeded_outer_promise".into(),
        (owner_slot, ptr.into()),
    );
    compiler.variable_hir_types.insert(
        "udeval_seeded_outer_promise".into(),
        HirType::Promise(Box::new(HirType::F64)),
    );

    let caught = compiler.context.append_basic_block(function, "udeval_seeded_catch");
    compiler.push_catch_target(caught);
    let loop_continue = compiler.context.append_basic_block(function, "udeval_seeded_loop_continue");
    let loop_break = compiler.context.append_basic_block(function, "udeval_seeded_loop_break");
    let boundary: std::collections::HashSet<PointerValue<'ctx>> = [owner_slot].into_iter().collect();
    let loop_scope = LoopScope {
        continue_target: loop_continue,
        break_target: loop_break,
        promise_boundary: boundary,
    };
    compiler.loop_scopes.push(loop_scope.clone());
    compiler.loop_promotion_scopes.push(LoopPromotionScope {
        preheader: entry,
        variables: ["udeval_retained_outer".to_string()]
            .into_iter().collect(),
        catches: CatchContext { scopes: compiler.catch_stack.clone() },
        stack_promise_slots: compiler.stack_promise_slots.clone(),
        outer_loops: vec![loop_scope],
    });

    let metadata = (0..5).map(|index|
        compiler.builder.build_alloca(ptr, &format!("udeval_catch_metadata_{index}"))
            .unwrap()).collect::<Vec<_>>();
    compiler.catch_native_text.insert("udeval_seeded_catch".into(), (
        metadata[0], metadata[1], metadata[2], metadata[3], metadata[4],
    ));
    let frame_slot = compiler.builder.build_alloca(ptr, "udeval_iteration_frame_slot").unwrap();
    compiler.for_iteration_frame_slots.insert("udeval_iteration".into(), frame_slot);
    compiler.active_async_completion = Some(ptr.const_null());
    compiler.builder.position_at_end(entry);
    entry
}

fn udeval_capture_conditional() -> HirExpr {
    let capture = HirParam { name: "udeval_transient_capture".into(), ty: HirType::F64 };
    let make_closure = || HirExpr::Lambda(
        vec![capture.clone()],
        Vec::new(),
        HirType::F64,
        Box::new(HirExpr::Var("udeval_transient_capture".into())),
    );
    HirExpr::Conditional(
        Box::new(HirExpr::Lit(HirLit::Bool(true))),
        Box::new(make_closure()),
        Box::new(make_closure()),
        HirType::Function(Vec::new(), Box::new(HirType::F64)),
    )
}

#[test]
fn discarded_native_promise_union_eval_then_and_errors() {
    let flat = udeval_flat_union_type();
    let three = udeval_three_union_type();
    let flat_promise = HirType::Promise(Box::new(HirType::F64));
    let members = vec![flat_promise.clone(), HirType::F64];

    // Effects-first / plain-second: the normal CFG releases the first
    // producer's exact selected token, then reaches the plain value producer.
    let plain_body = udeval_function_body(&udeval_compile_case(
        "udeval_effect_then_plain_second",
        HirType::Void,
        &[],
        HirStmt::Expr(udeval_eval_then(
            udeval_call("udeval_plain_first_union"),
            udeval_call("udeval_plain_second_value"),
        )),
        &[
            ("udeval_plain_first_union", flat.clone()),
            ("udeval_plain_second_value", HirType::F64),
        ],
    ), "udeval_effect_then_plain_second");
    let plain_blocks = udeval_blocks(&plain_body);
    let plain_first = udeval_call_result(&plain_body, "udeval_plain_first_union");
    let plain_first_trace = udeval_assert_union_destroy_identity(
        &plain_body, &plain_blocks, &plain_first, &[0],
    );
    assert_eq!(plain_body.lines().filter(|line| line.contains("@thaw_promise_destroy(")).count(),
        plain_first_trace.destroy_blocks.len(),
        "isolated plain-second function must contain only the first union's traced normal release");
    udeval_assert_result_destroy_only_on_normal_edge(
        &plain_blocks, "udeval_plain_first_union", &plain_first_trace,
    );
    udeval_assert_pending_edges_skip_later(
        &plain_blocks, "udeval_plain_first_union", "udeval_plain_second_value",
    );
    let plain_second = udeval_call_result(&plain_body, "udeval_plain_second_value");
    let plain_second_block = udeval_call_block(&plain_blocks, "udeval_plain_second_value");
    assert!(udeval_dominates(&plain_blocks, &plain_first_trace.merge, &plain_second_block.label),
        "plain second evaluation must be dominated by the first result's actual discard merge");
    assert!(!plain_body.lines().any(|line|
        line.contains("@thaw_promise_destroy(") && line.contains(&format!("ptr {plain_second})"))
    ), "plain F64 second result must not be passed to Promise destruction:\n{plain_body}");
    assert!(!plain_body.contains("@thaw_promise_retain("),
        "effects-only first plus plain second must not add a retain:\n{plain_body}");
    let (plain_second_normal, plain_second_exception) = udeval_call_edges(
        &plain_blocks, plain_second_block,
    );
    let plain_normal_returns = plain_blocks.iter().filter(|block|
        block.text.lines().any(|line| line.trim() == "ret void")
            && udeval_reaches(&plain_blocks, &plain_second_normal, &block.label)
    ).collect::<Vec<_>>();
    assert_eq!(plain_normal_returns.len(), 1,
        "normal second-call successor must reach the unique live function return");
    assert!(udeval_reaches(&plain_blocks, &plain_second_normal, &plain_normal_returns[0].label));
    assert!(!udeval_reaches(&plain_blocks, &plain_second_exception, &plain_normal_returns[0].label),
        "second call's exception successor must bypass its normal return continuation");

    // Two owned discarded results use distinct producer SSA identities. The
    // first merge dominates the second call on normal execution; both possible
    // Promise tags of the second result decode and destroy its own payload.
    let two_body = udeval_function_body(&udeval_compile_case(
        "udeval_two_owned_discarded_results",
        HirType::Void,
        &[],
        HirStmt::Expr(udeval_eval_then(
            udeval_call("udeval_two_first_flat"),
            udeval_call("udeval_two_second_three"),
        )),
        &[
            ("udeval_two_first_flat", flat.clone()),
            ("udeval_two_second_three", three.clone()),
        ],
    ), "udeval_two_owned_discarded_results");
    let two_blocks = udeval_blocks(&two_body);
    let first_result = udeval_call_result(&two_body, "udeval_two_first_flat");
    let first_trace = udeval_assert_union_destroy_identity(
        &two_body, &two_blocks, &first_result, &[0],
    );
    udeval_assert_result_destroy_only_on_normal_edge(
        &two_blocks, "udeval_two_first_flat", &first_trace,
    );
    let second_result = udeval_call_result(&two_body, "udeval_two_second_three");
    assert_ne!(first_result, second_result,
        "the two EvalThen producers must have distinct SSA result identities");
    let second_trace = udeval_assert_union_destroy_identity(
        &two_body, &two_blocks, &second_result, &[0, 2],
    );
    assert_eq!(two_body.lines().filter(|line| line.contains("@thaw_promise_destroy(")).count(),
        first_trace.destroy_blocks.len() + second_trace.destroy_blocks.len(),
        "isolated two-owned function must contain only its two traced union discards");
    let second_call_block = udeval_call_block(&two_blocks, "udeval_two_second_three");
    assert!(udeval_dominates(&two_blocks, &first_trace.merge, &second_call_block.label),
        "the second owned result must not be evaluated before the first's normal discard merge");
    udeval_assert_pending_edges_skip_later(
        &two_blocks, "udeval_two_first_flat", "udeval_two_second_three",
    );
    let (second_normal, second_exception) = udeval_call_edges(
        &two_blocks, second_call_block,
    );
    for destroy_block in &second_trace.destroy_blocks {
        assert!(udeval_reaches(&two_blocks, &second_normal, destroy_block),
            "second result's selected release belongs to its normal call successor");
        assert!(!udeval_reaches(&two_blocks, &second_exception, destroy_block),
            "pending exception from second producer must bypass its result discard");
    }
    assert!(!two_body.contains("@thaw_promise_retain("),
        "discarding two already-owned results must not retain either token:\n{two_body}");

    // A direct borrowed Promise parameter is the second EvalThen operand. It
    // needs no unguarded union extraction and no discard-only retain/destroy.
    let borrowed_promise = HirType::Promise(Box::new(HirType::F64));
    let borrowed_param = HirParam {
        name: "udeval_borrowed_promise".into(),
        ty: borrowed_promise,
    };
    let borrowed_body = udeval_function_body(&udeval_compile_case(
        "udeval_borrowed_second_alias",
        HirType::Void,
        &[borrowed_param],
        HirStmt::Expr(udeval_eval_then(
            udeval_call("udeval_borrowed_first_owned"),
            HirExpr::Var("udeval_borrowed_promise".into()),
        )),
        &[("udeval_borrowed_first_owned", flat.clone())],
    ), "udeval_borrowed_second_alias");
    let borrowed_blocks = udeval_blocks(&borrowed_body);
    let borrowed_first = udeval_call_result(&borrowed_body, "udeval_borrowed_first_owned");
    let borrowed_trace = udeval_assert_union_destroy_identity(
        &borrowed_body, &borrowed_blocks, &borrowed_first, &[0],
    );
    udeval_assert_result_destroy_only_on_normal_edge(
        &borrowed_blocks, "udeval_borrowed_first_owned", &borrowed_trace,
    );
    assert_eq!(borrowed_body.lines().filter(|line| line.contains("@thaw_promise_destroy(")).count(),
        borrowed_trace.destroy_blocks.len(),
        "borrowed-second function must contain only the first owned result's traced release");
    assert!(!borrowed_body.contains("@thaw_promise_retain("),
        "borrowed second Promise must not be acquired solely for discard:\n{borrowed_body}");
    let borrowed_loads = borrowed_body.lines().filter(|line|
        line.split_once("= load ptr, ptr ").is_some_and(|(_, operand)|
            operand.split(',').next().unwrap().trim() == "%udeval_borrowed_promise_cell"
        )
    ).collect::<Vec<_>>();
    assert_eq!(borrowed_loads.len(), 1,
        "borrowed second Promise must be loaded exactly once from its parameter cell:\n{borrowed_body}");
    let borrowed_load = borrowed_loads[0];
    let borrowed_value = udeval_ssa_result(borrowed_load);
    let borrowed_load_block = borrowed_blocks.iter().find(|block|
        block.text.lines().any(|line| line == borrowed_load)
    ).expect("borrowed second load must belong to a real normal CFG block");
    assert!(!borrowed_body.lines().any(|line|
        line.contains("@thaw_promise_destroy(") && line.contains(&format!("ptr {borrowed_value})"))
    ), "borrowed second alias must not be stolen by discard:\n{borrowed_body}");
    let borrowed_first_call = udeval_call_block(&borrowed_blocks, "udeval_borrowed_first_owned");
    let (borrowed_first_normal, borrowed_first_exception) =
        udeval_call_edges(&borrowed_blocks, borrowed_first_call);
    assert!(udeval_dominates(&borrowed_blocks, &borrowed_trace.merge, &borrowed_load_block.label),
        "borrowed second evaluation must follow the first's normal discard merge");
    assert!(udeval_reaches(&borrowed_blocks, &borrowed_first_normal, &borrowed_load_block.label));
    assert!(!udeval_reaches(&borrowed_blocks, &borrowed_first_exception, &borrowed_load_block.label),
        "pending exception from first call must bypass the borrowed second load");

    // The EvalThen prefix is hidden under both TypedClosure and UnionInject.
    // Peeling must keep source order, then preserve the second Promise's exact
    // SSA token through ptr-to-word, tagged payload, decode, and destruction.
    let wrapped = HirExpr::TypedClosure(
        flat.clone(),
        Box::new(HirExpr::UnionInject(
            Box::new(udeval_eval_then(
                udeval_call("udeval_hidden_first_union"),
                udeval_call("udeval_hidden_second_promise"),
            )),
            0,
            members,
        )),
    );
    let hidden_body = udeval_function_body(&udeval_compile_case(
        "udeval_wrapper_hidden_prefix",
        HirType::Void,
        &[],
        HirStmt::Expr(wrapped),
        &[
            ("udeval_hidden_first_union", flat.clone()),
            ("udeval_hidden_second_promise", flat_promise.clone()),
        ],
    ), "udeval_wrapper_hidden_prefix");
    let hidden_blocks = udeval_blocks(&hidden_body);
    let hidden_first = udeval_call_result(&hidden_body, "udeval_hidden_first_union");
    let hidden_first_trace = udeval_assert_union_destroy_identity(
        &hidden_body, &hidden_blocks, &hidden_first, &[0],
    );
    udeval_assert_result_destroy_only_on_normal_edge(
        &hidden_blocks, "udeval_hidden_first_union", &hidden_first_trace,
    );
    let hidden_second = udeval_assert_exact_promise_destroy_identity(
        &hidden_body, "udeval_hidden_second_promise",
    );
    let hidden_second_block = udeval_call_block(&hidden_blocks, "udeval_hidden_second_promise");
    assert!(udeval_dominates(&hidden_blocks, &hidden_first_trace.merge, &hidden_second_block.label),
        "hidden second must be evaluated only after first result's normal discard merge");
    udeval_assert_pending_edges_skip_later(
        &hidden_blocks, "udeval_hidden_first_union", "udeval_hidden_second_promise",
    );
    let (hidden_normal, hidden_exception) = udeval_call_edges(&hidden_blocks, hidden_second_block);
    let hidden_destroy_blocks = hidden_blocks.iter().filter(|block|
        block.text.lines().any(|line|
            line.contains("@thaw_promise_destroy(") && line.contains(&format!("ptr {hidden_second})"))
        )
    ).collect::<Vec<_>>();
    assert_eq!(hidden_destroy_blocks.len(), 1,
        "wrapper-hidden second SSA token must have one normal destroy block");
    assert_eq!(hidden_body.lines().filter(|line| line.contains("@thaw_promise_destroy(")).count(),
        hidden_first_trace.destroy_blocks.len() + 1,
        "isolated wrapper-hidden function must contain only its two traced releases");
    let hidden_destroy = hidden_destroy_blocks[0];
    assert!(udeval_reaches(&hidden_blocks, &hidden_normal, &hidden_destroy.label));
    assert!(!udeval_reaches(&hidden_blocks, &hidden_exception, &hidden_destroy.label),
        "pending exception from wrapper-hidden second call must bypass its discard");

    // Value-producing path: the effects-only first union is discarded, while
    // a plain F64 second call is returned as the same SSA value. This bounded
    // control deliberately avoids the separate Return(EvalThen(union)) HOLD.
    // P1 remains an explicit producer-route HOLD: Return(EvalThen(owned union,
    // owned union)) still lacks its separate return-only normalization proof.
    let value_body = udeval_function_body(&udeval_compile_case(
        "udeval_value_path_keeps_second_ssa",
        HirType::F64,
        &[],
        HirStmt::Return(Some(udeval_eval_then(
            udeval_call("udeval_value_first_owned_union"),
            udeval_call("udeval_value_second_f64"),
        ))),
        &[
            ("udeval_value_first_owned_union", flat.clone()),
            ("udeval_value_second_f64", HirType::F64),
        ],
    ), "udeval_value_path_keeps_second_ssa");
    let value_blocks = udeval_blocks(&value_body);
    let value_first = udeval_call_result(&value_body, "udeval_value_first_owned_union");
    let value_first_trace = udeval_assert_union_destroy_identity(
        &value_body, &value_blocks, &value_first, &[0],
    );
    assert_eq!(value_body.lines().filter(|line| line.contains("@thaw_promise_destroy(")).count(),
        value_first_trace.destroy_blocks.len(),
        "F64 value path must have no release beyond its effects-only first union");
    udeval_assert_result_destroy_only_on_normal_edge(
        &value_blocks, "udeval_value_first_owned_union", &value_first_trace,
    );
    udeval_assert_pending_edges_skip_later(
        &value_blocks, "udeval_value_first_owned_union", "udeval_value_second_f64",
    );
    let value_second = udeval_call_result(&value_body, "udeval_value_second_f64");
    let value_second_block = udeval_call_block(&value_blocks, "udeval_value_second_f64");
    assert!(udeval_dominates(&value_blocks, &value_first_trace.merge, &value_second_block.label),
        "value-producing EvalThen must evaluate its second only after first-discard merge");
    assert!(value_body.lines().any(|line|
        line.trim() == format!("ret double {value_second}")
    ), "the value path must return the exact second SSA result {value_second}:\n{value_body}");
    assert!(!value_body.contains("@thaw_promise_retain("),
        "discarding only the first result must not acquire or normalize the plain second value");
    let (value_second_normal, value_second_exception) = udeval_call_edges(
        &value_blocks, value_second_block,
    );
    let exact_second_return = format!("ret double {value_second}");
    let value_return_blocks = value_blocks.iter().filter(|block|
        block.text.lines().any(|line| line.trim() == exact_second_return)
    ).collect::<Vec<_>>();
    assert_eq!(value_return_blocks.len(), 1,
        "the second F64 result must have one exact live return block");
    assert!(udeval_reaches(&value_blocks, &value_second_normal, &value_return_blocks[0].label));
    assert!(!udeval_reaches(&value_blocks, &value_second_exception, &value_return_blocks[0].label),
        "pending exception from the second call must bypass the exact normal return");

    // Direct compile_expr value path with an owned union second result. The
    // manual LLVM return checks the returned BasicValueEnum without invoking
    // production Return(EvalThen) normalization; P1 remains HOLD separately.
    let direct_ir = udeval_compile_direct_owned_second_value();
    let direct_body = udeval_function_body(&direct_ir, "udeval_direct_expr_value_returns_second");
    let direct_blocks = udeval_blocks(&direct_body);
    let direct_first = udeval_call_result(&direct_body, "udeval_two_first_flat");
    let direct_first_trace = udeval_assert_union_destroy_identity(
        &direct_body, &direct_blocks, &direct_first, &[0],
    );
    assert_eq!(direct_body.lines().filter(|line| line.contains("@thaw_promise_destroy(")).count(),
        direct_first_trace.destroy_blocks.len(),
        "direct value path must release only the first result; the owned second is returned");
    udeval_assert_result_destroy_only_on_normal_edge(
        &direct_blocks, "udeval_two_first_flat", &direct_first_trace,
    );
    let direct_second = udeval_call_result(&direct_body, "udeval_two_second_three");
    assert_ne!(direct_first, direct_second,
        "direct value path must retain a distinct second-result SSA identity");
    let direct_second_call = udeval_call_block(&direct_blocks, "udeval_two_second_three");
    assert!(udeval_dominates(&direct_blocks, &direct_first_trace.merge, &direct_second_call.label),
        "owned second value must be evaluated after the first discard's live merge");
    udeval_assert_pending_edges_skip_later(
        &direct_blocks, "udeval_two_first_flat", "udeval_two_second_three",
    );
    let exact_direct_return = format!("ret {{ i8, i64 }} {direct_second}");
    let direct_return_blocks = direct_blocks.iter().filter(|block|
        block.text.lines().any(|line| line.trim() == exact_direct_return)
    ).collect::<Vec<_>>();
    assert_eq!(direct_return_blocks.len(), 1,
        "compile_expr's returned BasicValueEnum must be returned as the exact second SSA aggregate");
    let (direct_second_normal, direct_second_exception) =
        udeval_call_edges(&direct_blocks, direct_second_call);
    assert!(udeval_reaches(&direct_blocks, &direct_second_normal, &direct_return_blocks[0].label));
    assert!(!udeval_reaches(&direct_blocks, &direct_second_exception, &direct_return_blocks[0].label),
        "pending exception from second producer must bypass its direct return");
    let direct_second_operand = format!(" {direct_second},");
    assert!(!direct_body.lines().any(|line|
        line.contains("= extractvalue") && line.contains(&direct_second_operand)
    ), "owned second result must not be decoded for an effects-only discard:\n{direct_body}");
    assert!(!direct_body.contains("@thaw_promise_retain("),
        "direct value EvalThen must not retain the owned second result");

    // Pending-exception CFG control: a thrown first native result routes to
    // the catch successor, skipping both normal discard and the second effect.
    let exception_ir = udeval_compile_exception_case();
    let exception_body = udeval_function_body(&exception_ir, "udeval_exception_effects");
    let exception_blocks = udeval_blocks(&exception_body);
    let throwing_call = udeval_call_block(&exception_blocks, "udeval_throwing_first_union");
    let (normal, exceptional) = udeval_call_edges(&exception_blocks, throwing_call);
    assert_eq!(udeval_successors(throwing_call), vec![exceptional.clone(), normal.clone()],
        "true pending predicate successor must precede false normal successor");
    let exception_result = udeval_call_result(&exception_body, "udeval_throwing_first_union");
    let exception_trace = udeval_assert_union_destroy_identity(
        &exception_body, &exception_blocks, &exception_result, &[0],
    );
    assert_eq!(exception_body.lines().filter(|line| line.contains("@thaw_promise_destroy(")).count(),
        exception_trace.destroy_blocks.len(),
        "isolated exception CFG must have only the first result's normal discard site");
    udeval_assert_result_destroy_only_on_normal_edge(
        &exception_blocks, "udeval_throwing_first_union", &exception_trace,
    );
    let catch_edge = udeval_block(&exception_blocks, &exceptional);
    let catch_target = udeval_successors(catch_edge).first()
        .expect("pending exception cleanup must branch to catch").to_string();
    assert_eq!(catch_target, "udeval_caught_exception");
    let catch_block = udeval_block(&exception_blocks, &catch_target);
    assert!(catch_block.text.contains("ret void"));
    let later_effect = udeval_call_block(&exception_blocks, "udeval_after_first_effect");
    assert!(udeval_reaches(&exception_blocks, &normal, &later_effect.label));
    assert!(!udeval_reaches(&exception_blocks, &exceptional, &later_effect.label),
        "pending exception from first result must bypass the runtime second effect");
    assert!(!catch_edge.text.contains("@thaw_promise_destroy(")
            && !catch_edge.text.contains("@udeval_after_first_effect("),
        "exception successor must bypass normal first discard and second effect:\n{}", catch_edge.text);
    assert!(!exception_body.contains("@thaw_promise_retain("));

    // The successful conditional witness uses compile_expr's normal
    // conditional-value builder and must end at its live conditional_end;
    // it also proves capture prepromotion changes the seeded cell and arena
    // membership. Separately, the malformed Promise-valued EvalThen uses that
    // successful mutation as its first operand, then fails deterministically
    // in its typed second conditional. It is codegen-only and is never
    // verified, JIT-compiled, or executed as native code.
    let capture_expr = udeval_capture_conditional();
    let context = Context::create();
    let mut mutation_probe = HirCompiler::new(&context, "udeval_confirm_transient_mutation");
    let mutation_entry = udeval_seed_error_compiler(&mut mutation_probe, "udeval_mutation_probe");
    let seeded = udeval_snapshot_scope(&mutation_probe);
    let _captured_value = mutation_probe.compile_expr(&capture_expr).unwrap();
    let after_first = udeval_snapshot_scope(&mutation_probe);
    assert_ne!(
        after_first.variables["udeval_transient_capture"].0,
        seeded.variables["udeval_transient_capture"].0,
        "first EvalThen operand must actually replace the stack cell during capture prepromotion",
    );
    assert!(!seeded.arena_variables.contains("udeval_transient_capture"));
    assert!(after_first.arena_variables.contains("udeval_transient_capture"));
    let mutation_body = udeval_function_body(
        &mutation_probe.print_to_string(), "udeval_mutation_probe",
    );
    let mutation_blocks = udeval_blocks(&mutation_body);
    let then_block = udeval_block(&mutation_blocks, "conditional_then");
    let else_block = udeval_block(&mutation_blocks, "conditional_else");
    let merge_block = udeval_block(&mutation_blocks, "conditional_end");
    assert_eq!(udeval_successors(then_block), vec!["conditional_end".to_string()]);
    assert_eq!(udeval_successors(else_block), vec!["conditional_end".to_string()]);
    assert!(!merge_block.text.is_empty(), "successful merge must remain live");
    let probe_function = mutation_probe.module.get_function("udeval_mutation_probe").unwrap();
    let final_block = probe_function.get_basic_blocks().last().copied().unwrap();
    assert_eq!(mutation_probe.builder.get_insert_block(), Some(final_block),
        "successful conditional value compilation must leave insertion at its live merge");
    assert_ne!(mutation_probe.builder.get_insert_block(), Some(mutation_entry),
        "success must not receive the helper's error-only entry-block restoration");

    let error_context = Context::create();
    let mut error_probe = HirCompiler::new(&error_context, "udeval_error_restoration_probe");
    let error_entry = udeval_seed_error_compiler(&mut error_probe, "udeval_restoration_probe");
    let before_error = udeval_snapshot_scope(&error_probe);
    let promise_type = HirType::Promise(Box::new(HirType::F64));
    error_probe.function_return_types.insert("udeval_unselected_promise_arm".into(), promise_type.clone());
    error_probe.module.add_function(
        "udeval_unselected_promise_arm",
        error_probe.basic_type(&promise_type).unwrap().fn_type(&[], false),
        None,
    );
    let second_failure = HirExpr::Conditional(
        Box::new(HirExpr::Lit(HirLit::Bool(true))),
        Box::new(HirExpr::Var("udeval_missing_eval_then_binding".into())),
        Box::new(udeval_call("udeval_unselected_promise_arm")),
        promise_type.clone(),
    );
    let malformed = udeval_eval_then(capture_expr, second_failure);
    assert_eq!(error_probe.expr_hir_type(&malformed), Some(promise_type));
    assert!(error_probe.promise_expression_needs_discard(&malformed),
        "outer Promise-valued EvalThen must route through the actual effects-only discard caller");
    let error = error_probe.compile_stmt(&HirStmt::Expr(malformed))
        .expect_err("Promise-valued EvalThen must reach discard helper and fail at missing binding");
    assert!(error.contains("unknown variable `udeval_missing_eval_then_binding`"),
        "failure must be the deterministic second-operand missing-binding error: {error}");
    let failed_ir = error_probe.module.print_to_string().to_string();
    assert!(failed_ir.contains("udeval_transient_capture_cell"),
        "the first operand's arena-cell allocation proves transient mutation happened before error:\n{failed_ir}");
    assert_eq!(error_probe.builder.get_insert_block(), Some(error_entry),
        "outer error must restore the original builder insertion block");
    udeval_assert_scope_restored(&error_probe, &before_error);

    // This invokes only its separate verified valid module. The malformed
    // state-restoration module above remains codegen-only and is never JIT-run.
    udeval_run_mapped_exception_effect_probe();
}
