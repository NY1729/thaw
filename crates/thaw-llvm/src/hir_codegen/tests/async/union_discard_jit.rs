use std::collections::{BTreeSet, HashSet};
use std::ffi::c_void;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UdJitEvent {
    Factory(&'static str),
    Condition(&'static str),
    Retain(usize),
    Destroy(usize),
}

#[repr(C)]
struct UdJitTokenRecord {
    discriminator: u64,
}

static UD_JIT_TOKEN_FLAT_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x101 };
static UD_JIT_TOKEN_THREE_TAG0_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x202 };
static UD_JIT_TOKEN_THREE_TAG2_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x303 };
static UD_JIT_TOKEN_NESTED_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x404 };
static UD_JIT_TOKEN_OPTIONAL_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x808 };
static UD_JIT_TOKEN_NULLABLE_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x909 };
static UD_JIT_TOKEN_NULLISH_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0xa0a };
static UD_JIT_TOKEN_UNION_VALUE_OWNED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0xb0b };
static UD_JIT_TOKEN_BORROWED: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x505 };
static UD_JIT_TOKEN_PLAIN_F64_BITS: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x606 };
static UD_JIT_TOKEN_PLAIN_STR_BITS: UdJitTokenRecord = UdJitTokenRecord { discriminator: 0x707 };
static UD_JIT_EVENTS: Mutex<Vec<UdJitEvent>> = Mutex::new(Vec::new());
static UD_JIT_OUTER_MODE: Mutex<f64> = Mutex::new(0.0);
static UD_JIT_INNER_MODE: Mutex<f64> = Mutex::new(0.0);

fn udjit_token_address(record: &'static UdJitTokenRecord) -> usize {
    record as *const UdJitTokenRecord as usize
}

unsafe extern "C" fn udjit_stub_retain(token: *mut c_void) -> u8 {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Retain(token as usize));
    1
}

unsafe extern "C" fn udjit_stub_destroy(token: *mut c_void) {
    // Deliberately record only the address. These controls never hand a
    // synthetic token to the real Promise runtime or dereference it here.
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Destroy(token as usize));
}

unsafe extern "C" fn udjit_stub_flat_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("flat-token"));
    (&UD_JIT_TOKEN_FLAT_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_three_tag0_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("three-tag0-token"));
    (&UD_JIT_TOKEN_THREE_TAG0_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_three_tag2_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("three-tag2-token"));
    (&UD_JIT_TOKEN_THREE_TAG2_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_nested_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("nested-token"));
    (&UD_JIT_TOKEN_NESTED_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_optional_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("optional-token"));
    (&UD_JIT_TOKEN_OPTIONAL_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_nullable_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("nullable-token"));
    (&UD_JIT_TOKEN_NULLABLE_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_nullish_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("nullish-token"));
    (&UD_JIT_TOKEN_NULLISH_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_union_value_token() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("union-value-token"));
    (&UD_JIT_TOKEN_UNION_VALUE_OWNED as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_pointer_looking_f64() -> f64 {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("plain-f64-pointer-bits"));
    f64::from_bits(udjit_token_address(&UD_JIT_TOKEN_PLAIN_F64_BITS) as u64)
}

unsafe extern "C" fn udjit_stub_pointer_looking_str() -> *mut c_void {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Factory("plain-str-pointer-bits"));
    (&UD_JIT_TOKEN_PLAIN_STR_BITS as *const UdJitTokenRecord) as *mut c_void
}

unsafe extern "C" fn udjit_stub_outer_mode() -> f64 {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Condition("outer"));
    *UD_JIT_OUTER_MODE.lock().unwrap()
}

unsafe extern "C" fn udjit_stub_inner_mode() -> f64 {
    UD_JIT_EVENTS.lock().unwrap().push(UdJitEvent::Condition("inner"));
    *UD_JIT_INNER_MODE.lock().unwrap()
}

fn udjit_flat_union_type() -> HirType {
    HirType::Union(vec![HirType::Promise(Box::new(HirType::F64)), HirType::F64])
}

fn udjit_three_union_type() -> HirType {
    HirType::Union(vec![
        HirType::Promise(Box::new(HirType::F64)),
        HirType::Str,
        HirType::Promise(Box::new(HirType::I64)),
    ])
}

fn udjit_var(name: &str) -> HirExpr {
    HirExpr::Var(name.to_string())
}

fn udjit_call(name: &str, args: Vec<HirExpr>) -> HirExpr {
    HirExpr::Call(Box::new(udjit_var(name)), args)
}

fn udjit_mode_test(source: &str) -> HirExpr {
    HirExpr::BinOp(
        BinOp::Gt,
        Box::new(udjit_call(source, Vec::new())),
        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
    )
}

fn udjit_set_modes(outer: f64, inner: f64) {
    *UD_JIT_OUTER_MODE.lock().unwrap() = outer;
    *UD_JIT_INNER_MODE.lock().unwrap() = inner;
}

fn udjit_optional_value_of_some(payload: HirType, selected: HirExpr) -> HirExpr {
    HirExpr::OptionalValue(
        Box::new(HirExpr::TypedClosure(
            HirType::Optional(Box::new(payload.clone())),
            Box::new(HirExpr::OptionalSome(Box::new(selected), payload.clone())),
        )),
        payload,
    )
}

fn udjit_nullable_value_of_some(payload: HirType, selected: HirExpr) -> HirExpr {
    HirExpr::NullableValue(
        Box::new(HirExpr::TypedClosure(
            HirType::Nullable(Box::new(payload.clone())),
            Box::new(HirExpr::NullableSome(Box::new(selected), payload.clone())),
        )),
        payload,
    )
}

fn udjit_nullish_value_of_some(payload: HirType, selected: HirExpr) -> HirExpr {
    HirExpr::NullishValue(
        Box::new(HirExpr::TypedClosure(
            HirType::Nullish(Box::new(payload.clone())),
            Box::new(HirExpr::NullishSome(Box::new(selected), payload.clone())),
        )),
        payload,
    )
}

fn udjit_owned_borrowed_promise_selection(source: &str, promise_type: HirType) -> HirExpr {
    HirExpr::Conditional(
        Box::new(udjit_mode_test("udjit_test_outer_mode")),
        Box::new(udjit_call(source, Vec::new())),
        Box::new(udjit_var("borrowed")),
        promise_type,
    )
}

fn udjit_nested_owned_plain_borrowed_union_selection(
    source: &str,
    union_type: &HirType,
) -> HirExpr {
    let borrowed = udjit_inject(udjit_var("borrowed"), 0, union_type);
    let owned = udjit_inject(udjit_call(source, Vec::new()), 0, union_type);
    let nested = HirExpr::Conditional(
        Box::new(udjit_mode_test("udjit_test_inner_mode")),
        Box::new(owned),
        Box::new(udjit_call("udjit_owned_flat_plain_source", Vec::new())),
        union_type.clone(),
    );
    HirExpr::Conditional(
        Box::new(udjit_mode_test("udjit_test_outer_mode")),
        Box::new(nested),
        Box::new(borrowed),
        union_type.clone(),
    )
}

fn udjit_inject(value: HirExpr, tag: usize, members: &HirType) -> HirExpr {
    let HirType::Union(members) = members else { panic!("expected union type") };
    HirExpr::UnionInject(Box::new(value), tag, members.clone())
}

fn udjit_function(name: &str, params: Vec<HirParam>, ret: HirType, body: Vec<HirStmt>) -> HirFunction {
    HirFunction {
        name: name.to_string(),
        params,
        ret,
        is_async: false,
        body,
    }
}

fn udjit_param(name: &str, ty: HirType) -> HirParam {
    HirParam { name: name.to_string(), ty }
}

fn udjit_promise_f64() -> HirType {
    HirType::Promise(Box::new(HirType::F64))
}

fn udjit_promise_i64() -> HirType {
    HirType::Promise(Box::new(HirType::I64))
}

fn udjit_owned_union_source(
    name: &str,
    token_source: &str,
    member_type: HirType,
    tag: usize,
    union_type: HirType,
) -> HirFunction {
    let HirType::Union(union_members) = &union_type else { panic!("expected union return type") };
    assert_eq!(union_members.get(tag), Some(&member_type));
    let injected = udjit_inject(udjit_call(token_source, Vec::new()), tag, &union_type);
    udjit_function(
        name,
        Vec::new(),
        union_type,
        vec![HirStmt::Return(Some(injected))],
    )
}

fn udjit_void_probe(name: &str, expression: HirExpr) -> HirFunction {
    udjit_function(
        name,
        Vec::new(),
        HirType::Void,
        vec![HirStmt::Expr(expression), HirStmt::Return(None)],
    )
}

fn udjit_type_only_probe(name: &str, params: Vec<HirParam>, expression: HirExpr) -> HirFunction {
    udjit_function(
        name,
        params,
        HirType::Void,
        vec![HirStmt::Expr(expression), HirStmt::Return(None)],
    )
}

fn udjit_add_external<'ctx>(
    compiler: &mut HirCompiler<'ctx>,
    name: &str,
    result: HirType,
) -> Result<(), String> {
    let result_type = compiler.basic_type(&result)?;
    let function_type = result_type.fn_type(&[], false);
    compiler.module.add_function(name, function_type, Some(Linkage::External));
    compiler.function_return_types.insert(name.to_string(), result);
    Ok(())
}

fn udjit_compile_minimal<'ctx>(
    compiler: &mut HirCompiler<'ctx>,
    functions: &[HirFunction],
    external_functions: &[(&str, HirType)],
) -> Result<(), String> {
    compiler.declare_runtime_builtins();
    compiler.declare_exception_state();
    compiler.function_return_types = functions
        .iter()
        .map(|function| (function.name.clone(), function.ret.clone()))
        .collect();
    for (name, ret) in external_functions {
        udjit_add_external(compiler, name, ret.clone())?;
    }
    for function in functions {
        compiler.declare_function(function)?;
    }
    for function in functions {
        compiler.compile_function_body(function)?;
    }
    Ok(())
}

fn udjit_add_exception_reset<'ctx>(compiler: &HirCompiler<'ctx>) {
    let context = compiler.context;
    let reset = compiler.module.add_function(
        "udjit_reset_pending_exception_tuple",
        context.void_type().fn_type(&[], false),
        Some(Linkage::Internal),
    );
    let entry = context.append_basic_block(reset, "entry");
    let builder = context.create_builder();
    builder.position_at_end(entry);
    let pointers = [
        "__thaw_pending_exception",
        "__thaw_pending_rejection",
        "__thaw_pending_exception_native_text",
        "__thaw_pending_exception_object",
        "__thaw_pending_exception_native",
        "__thaw_pending_exception_aggregate_errors",
    ];
    let pointer_type = context.ptr_type(AddressSpace::default());
    for name in pointers {
        let slot = compiler.module.get_global(name).expect("declared exception pointer slot");
        builder.build_store(slot.as_pointer_value(), pointer_type.const_null()).unwrap();
    }
    for name in ["__thaw_pending_exception_value_tag", "__thaw_pending_exception_i64"] {
        let slot = compiler.module.get_global(name).expect("declared i64 exception slot");
        builder.build_store(slot.as_pointer_value(), context.i64_type().const_zero()).unwrap();
    }
    let f64_slot = compiler.module.get_global("__thaw_pending_exception_f64")
        .expect("declared f64 exception slot");
    builder.build_store(f64_slot.as_pointer_value(), context.f64_type().const_zero()).unwrap();
    let bool_slot = compiler.module.get_global("__thaw_pending_exception_bool")
        .expect("declared bool exception slot");
    builder.build_store(bool_slot.as_pointer_value(), context.bool_type().const_zero()).unwrap();
    builder.build_return(None).unwrap();
}

fn udjit_function_ir(ir: &str, name: &str) -> String {
    let marker = format!("@{name}(");
    let start = ir.lines().position(|line| line.starts_with("define ") && line.contains(&marker))
        .unwrap_or_else(|| panic!("missing generated function {name}:\n{ir}"));
    let lines = ir.lines().skip(start);
    let body = lines.take_while(|line| *line != "}").collect::<Vec<_>>().join("\n");
    body
}

fn udjit_ir_block(body: &str, label: &str) -> String {
    let wanted = format!("{label}:");
    let lines = body.lines().collect::<Vec<_>>();
    let start = lines.iter().position(|line| line.split(';').next().unwrap().trim() == wanted)
        .unwrap_or_else(|| panic!("missing block {label}:\n{body}"));
    let end = lines.iter().enumerate().skip(start + 1)
        .find(|(_, line)| line.split(';').next().unwrap().trim_end().ends_with(':'))
        .map(|(index, _)| index)
        .unwrap_or(lines.len());
    lines[start..end].join("\n")
}

fn udjit_operand_name(line: &str) -> String {
    line.split(" = ").next().unwrap_or("").trim().to_string()
}

fn udjit_branch_labels(line: &str) -> Vec<String> {
    line.match_indices("label %")
        .map(|(index, _)| {
            line[index + "label %".len()..]
                .split(|character: char| character == ',' || character == ')' || character.is_whitespace())
                .next().unwrap_or("").to_string()
        })
        .collect()
}

fn udjit_assert_selected_destroy_chain(ir: &str, consumer: &str, source: &str, promise_tags: &[u8]) {
    let body = udjit_function_ir(ir, consumer);
    let call_line = body.lines().find(|line| line.contains(&format!("@{source}(")))
        .unwrap_or_else(|| panic!("{consumer} does not call opaque union source {source}:\n{body}"));
    let union_ssa = udjit_operand_name(call_line);
    assert!(union_ssa.starts_with('%'), "expected named opaque result: {call_line}");
    let tag_line = body.lines().find(|line| {
        line.contains("extractvalue") && line.contains(&union_ssa) && line.contains(", 0")
    }).unwrap_or_else(|| panic!("no tag extraction from {union_ssa}:\n{body}"));
    let tag_ssa = udjit_operand_name(tag_line);
    let comparisons = body.lines().filter(|line| {
        line.contains("icmp eq i8") && line.contains(&tag_ssa)
    }).collect::<Vec<_>>();
    let actual_tags = comparisons.iter().map(|line| {
        line.rsplit_once(", ").expect("tag comparison has a literal right operand")
            .1.trim().parse::<u8>().expect("literal promise tag")
    }).collect::<Vec<_>>();
    assert_eq!(actual_tags.as_slice(), promise_tags, "wrong selected Promise-tag comparisons in {consumer}:\n{body}");
    assert_eq!(body.lines().filter(|line| line.contains("call void @thaw_promise_destroy(")).count(), promise_tags.len(),
        "expected one destroy callsite per direct Promise tag in {consumer}:\n{body}");
    assert_eq!(body.lines().filter(|line| line.contains("inttoptr i64")).count(), promise_tags.len(),
        "plain members must not be decoded as Promise pointers in {consumer}:\n{body}");

    for comparison in &comparisons {
        let literal = comparison.rsplit_once(", ").unwrap().1.trim().parse::<u8>().unwrap();
        let comparison_ssa = udjit_operand_name(comparison);
        let branch = body.lines().find(|line| line.contains("br i1") && line.contains(&comparison_ssa))
            .unwrap_or_else(|| panic!("tag {literal} comparison has no branch: {body}"));
        let labels = udjit_branch_labels(branch);
        assert_eq!(labels.len(), 2, "tag branch must select release or continue: {branch}");
        let selected = udjit_ir_block(&body, &labels[0]);
        let payload_line = selected.lines().find(|line| {
            line.contains("extractvalue") && line.contains(&union_ssa) && line.contains(", 1")
        }).unwrap_or_else(|| panic!("tag {literal} block does not extract the same union payload: {selected}"));
        let payload_ssa = udjit_operand_name(payload_line);
        let pointer_line = selected.lines().find(|line| {
            line.contains("inttoptr i64") && line.contains(&payload_ssa)
        }).unwrap_or_else(|| panic!("tag {literal} block does not decode its selected payload: {selected}"));
        let pointer_ssa = udjit_operand_name(pointer_line);
        let destroy = selected.lines().find(|line| line.contains("call void @thaw_promise_destroy("))
            .unwrap_or_else(|| panic!("tag {literal} block has no destructor: {selected}"));
        assert!(destroy.contains(&pointer_ssa),
            "tag {literal} destructor operand is not its selected payload pointer: {destroy}\n{selected}");
        assert!(selected.contains("br label %discarded_union_end"),
            "selected destructor path does not reach the live common merge: {selected}");
        assert!(body.contains("discarded_union_end:"), "missing common continuation: {body}");
    }
    let final_comparison = comparisons.last().expect("at least one Promise tag comparison");
    let final_branch = body.lines().find(|line| {
        line.contains("br i1") && line.contains(&udjit_operand_name(final_comparison))
    }).expect("final Promise tag comparison branch");
    let final_labels = udjit_branch_labels(final_branch);
    let final_fallback = udjit_ir_block(&body, &final_labels[1]);
    assert!(final_fallback.contains("br label %discarded_union_end"),
        "non-Promise fallthrough does not reach the live common merge: {final_fallback}");
    let merge = udjit_ir_block(&body, "discarded_union_end");
    assert!(merge.contains("ret void"), "merge has no live normal continuation: {merge}");
}

fn udjit_external_call_symbols(ir: &str, internal: &HashSet<String>) -> BTreeSet<String> {
    let mut external = BTreeSet::new();
    for line in ir.lines().filter(|line| line.contains(" call ") || line.trim_start().starts_with("call ")) {
        let at = line.find('@').unwrap_or_else(|| panic!("indirect or unrecognized call must be reviewed and mapped: {line}"));
        let name = line[at + 1..].chars()
            .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '$' | '-'))
            .collect::<String>();
        assert!(!name.is_empty(), "call instruction has no named callee: {line}");
        if !internal.contains(&name) {
            external.insert(name);
        }
    }
    external
}

#[test]
fn discarded_native_promise_union_selected_tokens_are_exact_and_closed() {
    let context = Context::create();
    let flat = udjit_flat_union_type();
    let three = udjit_three_union_type();
    let borrowed = udjit_promise_f64();
    let borrowed_i64 = udjit_promise_i64();
    let outer_test = udjit_mode_test("udjit_test_outer_mode");
    let inner_test = udjit_mode_test("udjit_test_inner_mode");
    let borrowed_flat = udjit_inject(udjit_var("borrowed"), 0, &flat);
    let owned_nested_flat = udjit_inject(
        udjit_call("udjit_test_token_nested_owned", Vec::new()),
        0,
        &flat,
    );
    let nested_inner = HirExpr::Conditional(
        Box::new(inner_test),
        Box::new(owned_nested_flat),
        Box::new(borrowed_flat.clone()),
        flat.clone(),
    );
    let wrapped_nested = HirExpr::TypedClosure(
        flat.clone(),
        Box::new(HirExpr::Conditional(
            Box::new(outer_test),
            Box::new(HirExpr::TypedClosure(flat.clone(), Box::new(nested_inner))),
            Box::new(borrowed_flat.clone()),
            flat.clone(),
        )),
    );
    let selected_owned_flat = udjit_inject(
        udjit_call("udjit_test_token_nested_owned", Vec::new()),
        0,
        &flat,
    );
    let selected_simple = HirExpr::Conditional(
        Box::new(udjit_mode_test("udjit_test_outer_mode")),
        Box::new(selected_owned_flat),
        Box::new(borrowed_flat),
        flat.clone(),
    );
    let selected_plain_flat = HirExpr::TypedClosure(
        flat.clone(),
        Box::new(HirExpr::Conditional(
            Box::new(udjit_mode_test("udjit_test_outer_mode")),
            Box::new(udjit_call("udjit_owned_flat_plain_source", Vec::new())),
            Box::new(udjit_call("udjit_owned_flat_plain_source", Vec::new())),
            flat.clone(),
        )),
    );
    let selected_plain_three = HirExpr::TypedClosure(
        three.clone(),
        Box::new(HirExpr::Conditional(
            Box::new(udjit_mode_test("udjit_test_outer_mode")),
            Box::new(udjit_call("udjit_owned_three_plain_source", Vec::new())),
            Box::new(udjit_call("udjit_owned_three_plain_source", Vec::new())),
            three.clone(),
        )),
    );
    let optional_payload = udjit_promise_f64();
    let optional_owned_value = HirExpr::OptionalValue(
        Box::new(HirExpr::OptionalSome(
            Box::new(udjit_call("udjit_test_token_optional_owned", Vec::new())),
            optional_payload.clone(),
        )),
        optional_payload.clone(),
    );
    let nullable_owned_value = HirExpr::NullableValue(
        Box::new(HirExpr::NullableSome(
            Box::new(udjit_call("udjit_test_token_nullable_owned", Vec::new())),
            optional_payload.clone(),
        )),
        optional_payload.clone(),
    );
    let nullish_owned_value = HirExpr::NullishValue(
        Box::new(HirExpr::NullishSome(
            Box::new(udjit_call("udjit_test_token_nullish_owned", Vec::new())),
            optional_payload.clone(),
        )),
        optional_payload.clone(),
    );
    let union_members = match &flat { HirType::Union(members) => members.clone(), _ => unreachable!() };
    let union_owned_value = HirExpr::UnionValue(
        Box::new(udjit_inject(
            udjit_call("udjit_test_token_union_value_owned", Vec::new()),
            0,
            &flat,
        )),
        0,
        union_members.clone(),
    );
    let optional_borrowed_value = HirExpr::OptionalValue(
        Box::new(HirExpr::OptionalSome(
            Box::new(udjit_var("borrowed")),
            optional_payload.clone(),
        )),
        optional_payload.clone(),
    );
    let nullable_borrowed_value = HirExpr::NullableValue(
        Box::new(HirExpr::NullableSome(
            Box::new(udjit_var("borrowed")),
            optional_payload.clone(),
        )),
        optional_payload.clone(),
    );
    let nullish_borrowed_value = HirExpr::NullishValue(
        Box::new(HirExpr::NullishSome(
            Box::new(udjit_var("borrowed")),
            optional_payload.clone(),
        )),
        optional_payload.clone(),
    );
    let union_borrowed_value = HirExpr::UnionValue(
        Box::new(udjit_inject(udjit_var("borrowed"), 0, &flat)),
        0,
        union_members,
    );
    let optional_promise_selection = udjit_optional_value_of_some(
        borrowed.clone(),
        udjit_owned_borrowed_promise_selection(
            "udjit_test_token_optional_owned",
            borrowed.clone(),
        ),
    );
    let nullable_promise_selection = udjit_nullable_value_of_some(
        borrowed.clone(),
        udjit_owned_borrowed_promise_selection(
            "udjit_test_token_nullable_owned",
            borrowed.clone(),
        ),
    );
    let nullish_promise_selection = udjit_nullish_value_of_some(
        borrowed.clone(),
        udjit_owned_borrowed_promise_selection(
            "udjit_test_token_nullish_owned",
            borrowed.clone(),
        ),
    );
    let optional_union_selection = udjit_optional_value_of_some(
        flat.clone(),
        udjit_nested_owned_plain_borrowed_union_selection(
            "udjit_test_token_optional_owned",
            &flat,
        ),
    );
    let nullable_union_selection = udjit_nullable_value_of_some(
        flat.clone(),
        udjit_nested_owned_plain_borrowed_union_selection(
            "udjit_test_token_nullable_owned",
            &flat,
        ),
    );
    let nullish_union_selection = udjit_nullish_value_of_some(
        flat.clone(),
        udjit_nested_owned_plain_borrowed_union_selection(
            "udjit_test_token_nullish_owned",
            &flat,
        ),
    );

    let functions = vec![
        udjit_owned_union_source(
            "udjit_owned_flat_source",
            "udjit_test_token_flat_owned",
            udjit_promise_f64(),
            0,
            flat.clone(),
        ),
        udjit_owned_union_source(
            "udjit_owned_flat_plain_source",
            "udjit_test_pointer_looking_f64",
            HirType::F64,
            1,
            flat.clone(),
        ),
        udjit_owned_union_source(
            "udjit_owned_three_tag0_source",
            "udjit_test_token_three_tag0_owned",
            udjit_promise_f64(),
            0,
            three.clone(),
        ),
        udjit_owned_union_source(
            "udjit_owned_three_plain_source",
            "udjit_test_pointer_looking_str",
            HirType::Str,
            1,
            three.clone(),
        ),
        udjit_owned_union_source(
            "udjit_owned_three_tag2_source",
            "udjit_test_token_three_tag2_owned",
            udjit_promise_i64(),
            2,
            three.clone(),
        ),
        udjit_function(
            "udjit_borrowed_return_flat",
            vec![udjit_param("borrowed", borrowed.clone())],
            flat.clone(),
            vec![HirStmt::Return(Some(udjit_inject(udjit_var("borrowed"), 0, &flat)))],
        ),
        udjit_function(
            "udjit_borrowed_return_promise",
            vec![udjit_param("borrowed", borrowed.clone())],
            borrowed.clone(),
            vec![HirStmt::Return(Some(udjit_var("borrowed")))],
        ),
        udjit_void_probe(
            "udjit_drop_owned_flat_call",
            udjit_call("udjit_owned_flat_source", Vec::new()),
        ),
        udjit_void_probe(
            "udjit_drop_owned_flat_plain_call",
            udjit_call("udjit_owned_flat_plain_source", Vec::new()),
        ),
        udjit_void_probe(
            "udjit_drop_owned_three_tag0_call",
            udjit_call("udjit_owned_three_tag0_source", Vec::new()),
        ),
        udjit_void_probe(
            "udjit_drop_owned_three_plain_call",
            udjit_call("udjit_owned_three_plain_source", Vec::new()),
        ),
        udjit_void_probe(
            "udjit_drop_owned_three_tag2_call",
            udjit_call("udjit_owned_three_tag2_source", Vec::new()),
        ),
        udjit_function(
            "udjit_drop_two_owned_calls",
            Vec::new(),
            HirType::Void,
            vec![
                HirStmt::Expr(udjit_call("udjit_owned_flat_source", Vec::new())),
                HirStmt::Expr(udjit_call("udjit_owned_three_tag2_source", Vec::new())),
                HirStmt::Return(None),
            ],
        ),
        udjit_type_only_probe(
            "udjit_drop_borrowed_exact_alias",
            vec![udjit_param("borrowed", borrowed.clone())],
            udjit_var("borrowed"),
        ),
        udjit_type_only_probe(
            "udjit_drop_borrowed_flat_union_alias",
            vec![udjit_param("borrowed", borrowed.clone())],
            udjit_inject(udjit_var("borrowed"), 0, &flat),
        ),
        udjit_type_only_probe(
            "udjit_drop_borrowed_three_tag2_union_alias",
            vec![udjit_param("borrowed", borrowed_i64.clone())],
            udjit_inject(udjit_var("borrowed"), 2, &three),
        ),
        udjit_type_only_probe(
            "udjit_drop_borrowed_flat_return_call",
            vec![udjit_param("borrowed", borrowed.clone())],
            udjit_call("udjit_borrowed_return_flat", vec![udjit_var("borrowed")]),
        ),
        udjit_type_only_probe(
            "udjit_drop_borrowed_promise_return_call",
            vec![udjit_param("borrowed", borrowed.clone())],
            udjit_call("udjit_borrowed_return_promise", vec![udjit_var("borrowed")]),
        ),
        udjit_function(
            "udjit_drop_selected_borrowed_or_owned",
            vec![udjit_param("borrowed", borrowed.clone())],
            HirType::Void,
            vec![
                HirStmt::Expr(selected_simple),
                HirStmt::Return(None),
            ],
        ),
        udjit_function(
            "udjit_drop_wrapped_nested_selection",
            vec![udjit_param("borrowed", borrowed.clone())],
            HirType::Void,
            vec![HirStmt::Expr(wrapped_nested), HirStmt::Return(None)],
        ),
        udjit_function(
            "udjit_drop_pointer_looking_flat_plain_selection",
            Vec::new(),
            HirType::Void,
            vec![HirStmt::Expr(selected_plain_flat), HirStmt::Return(None)],
        ),
        udjit_function(
            "udjit_drop_pointer_looking_three_plain_selection",
            Vec::new(),
            HirType::Void,
            vec![HirStmt::Expr(selected_plain_three), HirStmt::Return(None)],
        ),
        udjit_function(
            "udjit_drop_explicit_value_wrappers",
            Vec::new(),
            HirType::Void,
            vec![
                HirStmt::Expr(optional_owned_value),
                HirStmt::Expr(nullable_owned_value),
                HirStmt::Expr(nullish_owned_value),
                HirStmt::Expr(union_owned_value),
                HirStmt::Return(None),
            ],
        ),
        udjit_function(
            "udjit_drop_borrowed_explicit_value_wrappers",
            vec![udjit_param("borrowed", borrowed.clone())],
            HirType::Void,
            vec![
                HirStmt::Expr(optional_borrowed_value),
                HirStmt::Expr(nullable_borrowed_value),
                HirStmt::Expr(nullish_borrowed_value),
                HirStmt::Expr(union_borrowed_value),
                HirStmt::Return(None),
            ],
        ),
        udjit_type_only_probe(
            "udjit_optional_value_owned_borrowed_promise_selection",
            vec![udjit_param("borrowed", borrowed.clone())],
            optional_promise_selection,
        ),
        udjit_type_only_probe(
            "udjit_nullable_value_owned_borrowed_promise_selection",
            vec![udjit_param("borrowed", borrowed.clone())],
            nullable_promise_selection,
        ),
        udjit_type_only_probe(
            "udjit_nullish_value_owned_borrowed_promise_selection",
            vec![udjit_param("borrowed", borrowed.clone())],
            nullish_promise_selection,
        ),
        udjit_type_only_probe(
            "udjit_optional_value_nested_union_selection",
            vec![udjit_param("borrowed", borrowed.clone())],
            optional_union_selection,
        ),
        udjit_type_only_probe(
            "udjit_nullable_value_nested_union_selection",
            vec![udjit_param("borrowed", borrowed.clone())],
            nullable_union_selection,
        ),
        udjit_type_only_probe(
            "udjit_nullish_value_nested_union_selection",
            vec![udjit_param("borrowed", borrowed.clone())],
            nullish_union_selection,
        ),
    ];
    let external_functions = [
        ("udjit_test_token_flat_owned", udjit_promise_f64()),
        ("udjit_test_token_three_tag0_owned", udjit_promise_f64()),
        ("udjit_test_token_three_tag2_owned", udjit_promise_i64()),
        ("udjit_test_token_nested_owned", udjit_promise_f64()),
        ("udjit_test_token_optional_owned", udjit_promise_f64()),
        ("udjit_test_token_nullable_owned", udjit_promise_f64()),
        ("udjit_test_token_nullish_owned", udjit_promise_f64()),
        ("udjit_test_token_union_value_owned", udjit_promise_f64()),
        ("udjit_test_pointer_looking_f64", HirType::F64),
        ("udjit_test_pointer_looking_str", HirType::Str),
        ("udjit_test_outer_mode", HirType::F64),
        ("udjit_test_inner_mode", HirType::F64),
    ];
    let mut compiler = HirCompiler::new(&context, "udjit_union_discard_selected_tokens");
    udjit_compile_minimal(&mut compiler, &functions, &external_functions).unwrap();
    udjit_add_exception_reset(&compiler);
    compiler.module.verify().unwrap();

    let ir = compiler.print_to_string().to_string();
    udjit_assert_selected_destroy_chain(
        &ir,
        "udjit_drop_owned_flat_call",
        "udjit_owned_flat_source",
        &[0],
    );
    udjit_assert_selected_destroy_chain(
        &ir,
        "udjit_drop_owned_three_tag0_call",
        "udjit_owned_three_tag0_source",
        &[0, 2],
    );
    udjit_assert_selected_destroy_chain(
        &ir,
        "udjit_drop_owned_three_tag2_call",
        "udjit_owned_three_tag2_source",
        &[0, 2],
    );
    let two_owned_ir = udjit_function_ir(&ir, "udjit_drop_two_owned_calls");
    let first_release = two_owned_ir.find("@udjit_owned_flat_source()").unwrap();
    let second_release = two_owned_ir.find("@udjit_owned_three_tag2_source()").unwrap();
    assert!(first_release < second_release, "two owned expressions lost source order:\n{two_owned_ir}");
    assert_eq!(two_owned_ir.matches("call void @thaw_promise_destroy(").count(), 3,
        "flat plus both direct Promise tags in the three-member union need three branch callsites:\n{two_owned_ir}");
    let explicit_wrapper_ir = udjit_function_ir(&ir, "udjit_drop_explicit_value_wrappers");
    assert_eq!(explicit_wrapper_ir.matches("call void @thaw_promise_destroy(").count(), 4,
        "four proven explicit value-extraction wrappers each need one exact-Promise release site:\n{explicit_wrapper_ir}");

    let mut internal = functions.iter().map(|function| function.name.clone()).collect::<HashSet<_>>();
    internal.insert("udjit_reset_pending_exception_tuple".to_string());
    let external_calls = udjit_external_call_symbols(&ir, &internal);
    let expected_external_calls = BTreeSet::from([
        "thaw_promise_destroy".to_string(),
        "thaw_promise_retain".to_string(),
        "udjit_test_pointer_looking_f64".to_string(),
        "udjit_test_pointer_looking_str".to_string(),
        "udjit_test_token_flat_owned".to_string(),
        "udjit_test_token_nested_owned".to_string(),
        "udjit_test_token_optional_owned".to_string(),
        "udjit_test_token_nullable_owned".to_string(),
        "udjit_test_token_nullish_owned".to_string(),
        "udjit_test_token_union_value_owned".to_string(),
        "udjit_test_token_three_tag0_owned".to_string(),
        "udjit_test_token_three_tag2_owned".to_string(),
        "udjit_test_outer_mode".to_string(),
        "udjit_test_inner_mode".to_string(),
    ]);
    assert_eq!(external_calls, expected_external_calls,
        "generated external-call closure changed; add an ABI-correct non-dereferencing stub or reject before JIT invocation:\n{ir}");

    let engine = compiler.module.create_jit_execution_engine(OptimizationLevel::None).unwrap();
    let retain = compiler.module.get_function("thaw_promise_retain").unwrap();
    assert_eq!(retain.get_type().get_param_types().len(), 1);
    let retain_params = retain.get_type().get_param_types();
    assert!(retain_params[0].is_pointer_type(), "retain spy must receive the opaque token pointer");
    let retain_result = retain.get_type().get_return_type().unwrap();
    assert!(retain_result.is_int_type());
    assert_eq!(retain_result.into_int_type().get_bit_width(), 8);
    engine.add_global_mapping(&retain, udjit_stub_retain as usize);
    let destroy = compiler.module.get_function("thaw_promise_destroy").unwrap();
    let destroy_params = destroy.get_type().get_param_types();
    assert_eq!(destroy_params.len(), 1);
    assert!(destroy_params[0].is_pointer_type(), "destroy spy must receive the opaque token pointer");
    assert!(destroy.get_type().get_return_type().is_none());
    engine.add_global_mapping(&destroy, udjit_stub_destroy as usize);
    for (symbol, address) in [
        ("udjit_test_token_flat_owned", udjit_stub_flat_token as usize),
        ("udjit_test_token_three_tag0_owned", udjit_stub_three_tag0_token as usize),
        ("udjit_test_token_three_tag2_owned", udjit_stub_three_tag2_token as usize),
        ("udjit_test_token_nested_owned", udjit_stub_nested_token as usize),
        ("udjit_test_token_optional_owned", udjit_stub_optional_token as usize),
        ("udjit_test_token_nullable_owned", udjit_stub_nullable_token as usize),
        ("udjit_test_token_nullish_owned", udjit_stub_nullish_token as usize),
        ("udjit_test_token_union_value_owned", udjit_stub_union_value_token as usize),
        ("udjit_test_pointer_looking_f64", udjit_stub_pointer_looking_f64 as usize),
        ("udjit_test_pointer_looking_str", udjit_stub_pointer_looking_str as usize),
        ("udjit_test_outer_mode", udjit_stub_outer_mode as usize),
        ("udjit_test_inner_mode", udjit_stub_inner_mode as usize),
    ] {
        let function = compiler.module.get_function(symbol).expect("closed external-call map entry");
        assert!(function.get_type().get_param_types().is_empty(), "unexpected ABI args for {symbol}");
        let return_type = function.get_type().get_return_type().expect("test-only provider returns a value");
        if matches!(symbol, "udjit_test_pointer_looking_f64" | "udjit_test_outer_mode" | "udjit_test_inner_mode") {
            assert!(return_type.is_float_type(), "F64 pointer-looking fixture ABI changed");
            assert_eq!(return_type.into_float_type().get_bit_width(), 64);
        } else {
            assert!(return_type.is_pointer_type(), "token/Str fixture ABI changed for {symbol}");
        }
        engine.add_global_mapping(&function, address);
    }

    type UdJitVoidProbe = unsafe extern "C" fn();
    type UdJitBorrowedProbe = unsafe extern "C" fn(*mut c_void);
    type UdJitSelectedProbe = unsafe extern "C" fn(*mut c_void);
    unsafe {
        let reset = engine.get_function::<UdJitVoidProbe>("udjit_reset_pending_exception_tuple").unwrap();
        let drop_flat = engine.get_function::<UdJitVoidProbe>("udjit_drop_owned_flat_call").unwrap();
        let drop_flat_plain = engine.get_function::<UdJitVoidProbe>("udjit_drop_owned_flat_plain_call").unwrap();
        let drop_three_tag0 = engine.get_function::<UdJitVoidProbe>("udjit_drop_owned_three_tag0_call").unwrap();
        let drop_three_tag2 = engine.get_function::<UdJitVoidProbe>("udjit_drop_owned_three_tag2_call").unwrap();
        let drop_three_plain = engine.get_function::<UdJitVoidProbe>("udjit_drop_owned_three_plain_call").unwrap();
        let drop_two = engine.get_function::<UdJitVoidProbe>("udjit_drop_two_owned_calls").unwrap();
        let drop_explicit_wrappers = engine.get_function::<UdJitVoidProbe>("udjit_drop_explicit_value_wrappers").unwrap();
        let drop_borrowed_exact = engine.get_function::<UdJitBorrowedProbe>("udjit_drop_borrowed_exact_alias").unwrap();
        let drop_borrowed_flat = engine.get_function::<UdJitBorrowedProbe>("udjit_drop_borrowed_flat_union_alias").unwrap();
        let drop_borrowed_three = engine.get_function::<UdJitBorrowedProbe>("udjit_drop_borrowed_three_tag2_union_alias").unwrap();
        let drop_borrowed_wrappers = engine.get_function::<UdJitBorrowedProbe>("udjit_drop_borrowed_explicit_value_wrappers").unwrap();
        let drop_borrowed_flat_return = engine.get_function::<UdJitBorrowedProbe>("udjit_drop_borrowed_flat_return_call").unwrap();
        let drop_borrowed_promise_return = engine.get_function::<UdJitBorrowedProbe>("udjit_drop_borrowed_promise_return_call").unwrap();
        let drop_optional_promise_selection = engine.get_function::<UdJitBorrowedProbe>("udjit_optional_value_owned_borrowed_promise_selection").unwrap();
        let drop_nullable_promise_selection = engine.get_function::<UdJitBorrowedProbe>("udjit_nullable_value_owned_borrowed_promise_selection").unwrap();
        let drop_nullish_promise_selection = engine.get_function::<UdJitBorrowedProbe>("udjit_nullish_value_owned_borrowed_promise_selection").unwrap();
        let drop_optional_union_selection = engine.get_function::<UdJitBorrowedProbe>("udjit_optional_value_nested_union_selection").unwrap();
        let drop_nullable_union_selection = engine.get_function::<UdJitBorrowedProbe>("udjit_nullable_value_nested_union_selection").unwrap();
        let drop_nullish_union_selection = engine.get_function::<UdJitBorrowedProbe>("udjit_nullish_value_nested_union_selection").unwrap();
        let drop_selected = engine.get_function::<UdJitSelectedProbe>("udjit_drop_selected_borrowed_or_owned").unwrap();
        let drop_nested = engine.get_function::<UdJitSelectedProbe>("udjit_drop_wrapped_nested_selection").unwrap();
        let drop_flat_plain_selected = engine.get_function::<UdJitVoidProbe>("udjit_drop_pointer_looking_flat_plain_selection").unwrap();
        let drop_three_plain_selected = engine.get_function::<UdJitVoidProbe>("udjit_drop_pointer_looking_three_plain_selection").unwrap();

        let invoke = |function: &inkwell::execution_engine::JitFunction<'_, UdJitVoidProbe>| {
            unsafe { reset.call(); }
            UD_JIT_EVENTS.lock().unwrap().clear();
            unsafe { function.call(); }
            UD_JIT_EVENTS.lock().unwrap().clone()
        };
        let flat_owned = udjit_token_address(&UD_JIT_TOKEN_FLAT_OWNED);
        let three_tag0_owned = udjit_token_address(&UD_JIT_TOKEN_THREE_TAG0_OWNED);
        let three_tag2_owned = udjit_token_address(&UD_JIT_TOKEN_THREE_TAG2_OWNED);
        let nested_owned = udjit_token_address(&UD_JIT_TOKEN_NESTED_OWNED);
        let borrowed_token = udjit_token_address(&UD_JIT_TOKEN_BORROWED) as *mut c_void;
        let plain_f64_bits = f64::from_bits(udjit_token_address(&UD_JIT_TOKEN_PLAIN_F64_BITS) as u64);
        let invoke_borrowed = |function: &inkwell::execution_engine::JitFunction<'_, UdJitBorrowedProbe>, outer: f64, inner: f64| {
            unsafe { reset.call(); }
            UD_JIT_EVENTS.lock().unwrap().clear();
            udjit_set_modes(outer, inner);
            unsafe { function.call(borrowed_token); }
            UD_JIT_EVENTS.lock().unwrap().clone()
        };

        assert_eq!(invoke(&drop_flat), vec![UdJitEvent::Factory("flat-token"), UdJitEvent::Destroy(flat_owned)]);
        assert_eq!(invoke(&drop_flat_plain), vec![UdJitEvent::Factory("plain-f64-pointer-bits")],
            "tag 1 F64 carrying pointer-looking bits must remain plain");
        assert_eq!(invoke(&drop_three_tag0), vec![UdJitEvent::Factory("three-tag0-token"), UdJitEvent::Destroy(three_tag0_owned)]);
        assert_eq!(invoke(&drop_three_tag2), vec![UdJitEvent::Factory("three-tag2-token"), UdJitEvent::Destroy(three_tag2_owned)]);
        assert_eq!(invoke(&drop_three_plain), vec![UdJitEvent::Factory("plain-str-pointer-bits")],
            "tag 1 Str carrying pointer-looking bits must remain plain");
        assert_eq!(invoke(&drop_two), vec![
            UdJitEvent::Factory("flat-token"),
            UdJitEvent::Destroy(flat_owned),
            UdJitEvent::Factory("three-tag2-token"),
            UdJitEvent::Destroy(three_tag2_owned),
        ], "two independently selected output tokens must be destroyed once each and in source order");
        assert_eq!(invoke(&drop_explicit_wrappers), vec![
            UdJitEvent::Factory("optional-token"),
            UdJitEvent::Destroy(udjit_token_address(&UD_JIT_TOKEN_OPTIONAL_OWNED)),
            UdJitEvent::Factory("nullable-token"),
            UdJitEvent::Destroy(udjit_token_address(&UD_JIT_TOKEN_NULLABLE_OWNED)),
            UdJitEvent::Factory("nullish-token"),
            UdJitEvent::Destroy(udjit_token_address(&UD_JIT_TOKEN_NULLISH_OWNED)),
            UdJitEvent::Factory("union-value-token"),
            UdJitEvent::Destroy(udjit_token_address(&UD_JIT_TOKEN_UNION_VALUE_OWNED)),
        ], "explicit Optional/Nullable/Nullish/Union value wrappers must preserve the selected external token");

        for probe in [drop_borrowed_exact, drop_borrowed_flat, drop_borrowed_three, drop_borrowed_wrappers] {
            reset.call();
            UD_JIT_EVENTS.lock().unwrap().clear();
            probe.call(borrowed_token);
            let events = UD_JIT_EVENTS.lock().unwrap().clone();
            assert!(events.is_empty(), "borrowed alias discard added a retain or destroy: {events:?}");
        }
        for probe in [drop_borrowed_flat_return, drop_borrowed_promise_return] {
            reset.call();
            UD_JIT_EVENTS.lock().unwrap().clear();
            probe.call(borrowed_token);
            assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![
                UdJitEvent::Retain(borrowed_token as usize),
                UdJitEvent::Destroy(borrowed_token as usize),
            ], "a borrowed native return must acquire its output token, then the consumer retires exactly that token");
        }

        for (probe, factory, token) in [
            (&drop_optional_promise_selection, "optional-token", udjit_token_address(&UD_JIT_TOKEN_OPTIONAL_OWNED)),
            (&drop_nullable_promise_selection, "nullable-token", udjit_token_address(&UD_JIT_TOKEN_NULLABLE_OWNED)),
            (&drop_nullish_promise_selection, "nullish-token", udjit_token_address(&UD_JIT_TOKEN_NULLISH_OWNED)),
        ] {
            assert_eq!(invoke_borrowed(probe, 1.0, 0.0), vec![
                UdJitEvent::Condition("outer"),
                UdJitEvent::Factory(factory),
                UdJitEvent::Destroy(token),
            ], "{factory} Some/Value selected owner must be destroyed exactly once");
            assert_eq!(invoke_borrowed(probe, 0.0, 1.0), vec![UdJitEvent::Condition("outer")],
                "{factory} Some/Value borrowed selection must short-circuit its factory and retain/destroy nothing");
        }
        for (probe, factory, token) in [
            (&drop_optional_union_selection, "optional-token", udjit_token_address(&UD_JIT_TOKEN_OPTIONAL_OWNED)),
            (&drop_nullable_union_selection, "nullable-token", udjit_token_address(&UD_JIT_TOKEN_NULLABLE_OWNED)),
            (&drop_nullish_union_selection, "nullish-token", udjit_token_address(&UD_JIT_TOKEN_NULLISH_OWNED)),
        ] {
            assert_eq!(invoke_borrowed(probe, 1.0, 1.0), vec![
                UdJitEvent::Condition("outer"),
                UdJitEvent::Condition("inner"),
                UdJitEvent::Factory(factory),
                UdJitEvent::Destroy(token),
            ], "{factory} wrapped-union owner selected tag 0 must be destroyed exactly once");
            assert_eq!(invoke_borrowed(probe, 1.0, 0.0), vec![
                UdJitEvent::Condition("outer"),
                UdJitEvent::Condition("inner"),
                UdJitEvent::Factory("plain-f64-pointer-bits"),
            ], "{factory} wrapped-union plain tag 1 must not be released or retained");
            assert_eq!(invoke_borrowed(probe, 0.0, 1.0), vec![UdJitEvent::Condition("outer")],
                "{factory} wrapped-union borrowed outer arm must skip the inner condition and factory");
        }

        reset.call();
        UD_JIT_EVENTS.lock().unwrap().clear();
        udjit_set_modes(1.0, 0.0);
        drop_selected.call(borrowed_token);
        assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![UdJitEvent::Condition("outer"), UdJitEvent::Factory("nested-token"), UdJitEvent::Destroy(nested_owned)],
            "selected owned arm did not release its own token");
        reset.call();
        UD_JIT_EVENTS.lock().unwrap().clear();
        udjit_set_modes(0.0, 1.0);
        drop_selected.call(borrowed_token);
        assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![UdJitEvent::Condition("outer")],
            "borrowed alternate was destroyed or the condition was re-evaluated");

        reset.call();
        UD_JIT_EVENTS.lock().unwrap().clear();
        udjit_set_modes(1.0, 1.0);
        drop_nested.call(borrowed_token);
        assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![UdJitEvent::Condition("outer"), UdJitEvent::Condition("inner"), UdJitEvent::Factory("nested-token"), UdJitEvent::Destroy(nested_owned)],
            "nested TypedClosure-selected owned leaf was not retired exactly once");
        reset.call();
        UD_JIT_EVENTS.lock().unwrap().clear();
        udjit_set_modes(1.0, 0.0);
        drop_nested.call(borrowed_token);
        assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![UdJitEvent::Condition("outer"), UdJitEvent::Condition("inner")],
            "nested borrowed leaf was released or a condition was re-evaluated");
        reset.call();
        UD_JIT_EVENTS.lock().unwrap().clear();
        udjit_set_modes(0.0, 1.0);
        drop_nested.call(borrowed_token);
        assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![UdJitEvent::Condition("outer")],
            "outer borrowed alternate was released or inner condition was evaluated eagerly");

        for mode in [0.0, 1.0] {
            reset.call();
            UD_JIT_EVENTS.lock().unwrap().clear();
            udjit_set_modes(mode, 0.0);
            drop_flat_plain_selected.call();
            assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![UdJitEvent::Condition("outer"), UdJitEvent::Factory("plain-f64-pointer-bits")],
                "flat pointer-looking F64 selected by conditional was decoded as a Promise");
            reset.call();
            UD_JIT_EVENTS.lock().unwrap().clear();
            udjit_set_modes(mode, 0.0);
            drop_three_plain_selected.call();
            assert_eq!(*UD_JIT_EVENTS.lock().unwrap(), vec![UdJitEvent::Condition("outer"), UdJitEvent::Factory("plain-str-pointer-bits")],
                "three-member pointer-looking Str selected by conditional was decoded as a Promise");
        }

        // Prove the F64 plain payload is exactly a token-record address before
        // using it as a pointer-looking non-Promise arm.
        assert_eq!(plain_f64_bits.to_bits(), udjit_token_address(&UD_JIT_TOKEN_PLAIN_F64_BITS) as u64);
    }
}
