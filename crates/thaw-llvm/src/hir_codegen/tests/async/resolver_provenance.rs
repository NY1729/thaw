fn resolver_provenance_compile_source_to_ir(source: &str) -> String {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "resolver_native_string_provenance");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    compiler.print_to_string()
}

fn resolver_provenance_defined_function_bodies(ir: &str) -> Vec<&str> {
    ir.split("define ")
        .skip(1)
        .filter_map(|definition| definition.split_once("\n}").map(|(body, _)| body))
        .collect()
}

fn resolver_provenance_reject_resolver_bodies(ir: &str) -> Vec<&str> {
    resolver_provenance_defined_function_bodies(ir)
        .into_iter()
        .filter(|body| body.starts_with("internal void @__thaw_promise_reject_"))
        .collect()
}

fn resolver_provenance_mixed_producer_source() -> &'static str {
    r#"
        function main(): void {
            const nativeReason: number[] = [1];
            Promise.reject<number[]>(nativeReason);
            new Promise<number>((resolve, reject) => reject("plain string"));
        }
    "#
}

#[test]
fn plain_string_resolver_clears_native_descriptor_from_real_typed_publisher() {
    let ir = resolver_provenance_compile_source_to_ir(resolver_provenance_mixed_producer_source());
    let functions = resolver_provenance_defined_function_bodies(&ir);
    let native_publishers = functions
        .iter()
        .filter(|body| {
            body.contains("call ptr @thaw_exception_native_provenance_new(i64 21")
                && body.contains(
                    "store ptr %pending_native_exception_descriptor, ptr @__thaw_pending_exception_native,",
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(native_publishers.len(), 1, "{ir}");

    let resolvers = resolver_provenance_reject_resolver_bodies(&ir);
    assert_eq!(resolvers.len(), 2, "{ir}");
    let plain_string_tag_store = format!(
        "store i64 4, ptr @{PENDING_EXCEPTION_VALUE_TAG_SYMBOL},"
    );
    let plain_string_resolver = resolvers
        .into_iter()
        .find(|body| body.contains(plain_string_tag_store.as_str()))
        .expect(&ir);
    let lines = plain_string_resolver.lines().collect::<Vec<_>>();
    let descriptor_clear = lines
        .iter()
        .position(|line| line.contains("store ptr null, ptr @__thaw_pending_exception_native,"))
        .expect(&ir);
    let plain_string_tag = lines
        .iter()
        .position(|line| line.contains(plain_string_tag_store.as_str()))
        .expect(&ir);
    let text_provenance = lines
        .iter()
        .position(|line| {
            line.contains("store ptr %")
                && line.contains("ptr @__thaw_pending_exception_native_text,")
        })
        .expect(&ir);
    let descriptor_load = lines
        .iter()
        .position(|line| {
            line.contains("load ptr, ptr @__thaw_pending_exception_native,")
        })
        .expect(&ir);
    let reject_call = lines
        .iter()
        .position(|line| {
            line.contains("call i8 @thaw_promise_reject_typed_with_native_provenance(")
        })
        .expect(&ir);

    assert!(
        descriptor_clear < plain_string_tag
            && plain_string_tag < text_provenance
            && text_provenance < descriptor_load
            && descriptor_load < reject_call,
        "{ir}"
    );
}

#[test]
fn typed_native_reason_resolver_preserves_its_published_descriptor() {
    let ir = resolver_provenance_compile_source_to_ir(resolver_provenance_mixed_producer_source());
    let functions = resolver_provenance_defined_function_bodies(&ir);
    assert!(
        functions.iter().any(|body| {
            body.contains("call ptr @thaw_exception_native_provenance_new(i64 21")
                && body.contains(
                    "store ptr %pending_native_exception_descriptor, ptr @__thaw_pending_exception_native,",
                )
        }),
        "{ir}"
    );

    let resolvers = resolver_provenance_reject_resolver_bodies(&ir);
    let typed_native_resolver = resolvers
        .into_iter()
        .find(|body| {
            body.contains("call i8 @thaw_promise_reject_typed_with_native_provenance(")
                && !body.contains("store ptr null, ptr @__thaw_pending_exception_native,")
        })
        .expect(&ir);
    let descriptor_load = typed_native_resolver
        .find("load ptr, ptr @__thaw_pending_exception_native,")
        .expect(&ir);
    let reject_call = typed_native_resolver
        .find("call i8 @thaw_promise_reject_typed_with_native_provenance(")
        .expect(&ir);
    assert!(descriptor_load < reject_call, "{ir}");
}

fn resolver_provenance_bind_pointer<'ctx>(
    compiler: &mut HirCompiler<'ctx>,
    binding: &str,
    global_name: &str,
    contents: &str,
) -> HirExpr {
    let pointer = compiler.context.ptr_type(AddressSpace::default());
    let value = compiler
        .builder
        .build_global_string_ptr(contents, global_name)
        .unwrap()
        .as_pointer_value();
    let slot = compiler
        .builder
        .build_alloca(pointer, &format!("{binding}_slot"))
        .unwrap();
    compiler.builder.build_store(slot, value).unwrap();
    compiler
        .variables
        .insert(binding.to_owned(), (slot, pointer.into()));
    HirExpr::Var(binding.to_owned())
}

#[test]
fn pending_rethrow_keeps_text_and_descriptor_on_their_own_channels() {
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "resolver_rethrow_channel_operands");
    compiler.declare_exception_state();
    let function = compiler.module.add_function(
        "resolver_provenance_pending_rethrow",
        context.void_type().fn_type(&[], false),
        None,
    );
    let entry = context.append_basic_block(function, "entry");
    compiler.builder.position_at_end(entry);
    let original = resolver_provenance_bind_pointer(
        &mut compiler,
        "reason_text",
        "resolver_reason_text",
        "same rejection bytes",
    );
    let native_text = resolver_provenance_bind_pointer(
        &mut compiler,
        "trusted_native_text",
        "resolver_trusted_native_text",
        "trusted native display",
    );
    let aggregate = resolver_provenance_bind_pointer(
        &mut compiler,
        "aggregate_errors",
        "resolver_aggregate_errors",
        "aggregate sentinel",
    );
    let object = resolver_provenance_bind_pointer(
        &mut compiler,
        "object_reason",
        "resolver_object_reason",
        "object sentinel",
    );
    let native_descriptor = resolver_provenance_bind_pointer(
        &mut compiler,
        "native_descriptor",
        "resolver_native_descriptor",
        "descriptor sentinel",
    );
    let pending_rethrow = HirExpr::Call(
        Box::new(HirExpr::Var(
            "@@thaw_rethrow_pending_exception".to_owned(),
        )),
        vec![
            original.clone(),
            original,
            native_text,
            aggregate,
            HirExpr::Lit(HirLit::I64(7)),
            HirExpr::Lit(HirLit::F64(8.5)),
            HirExpr::Lit(HirLit::I64(9)),
            HirExpr::Lit(HirLit::Bool(true)),
            object,
            native_descriptor,
        ],
    );

    // These globals are deliberately only code-generation sentinels. The
    // test inspects the emitted instructions and never executes the pointers.
    compiler.compile_throw_text(&pending_rethrow).unwrap();
    compiler.builder.build_return(None).unwrap();
    compiler.module.verify().unwrap();
    let ir = compiler.print_to_string();
    let body = resolver_provenance_defined_function_bodies(&ir)
        .into_iter()
        .find(|body| body.starts_with("void @resolver_provenance_pending_rethrow("))
        .expect(&ir);
    let lines = body.lines().collect::<Vec<_>>();
    let native_text_select = lines
        .iter()
        .find(|line| {
            line.contains("%rethrow_native_text = select")
                && line.contains("i1 %same_rejection_binding")
                && line.contains("ptr %trusted_native_text")
        })
        .expect(&ir);
    let native_descriptor_select = lines
        .iter()
        .find(|line| {
            line.contains("%rethrow_native_exception = select")
                && line.contains("i1 %same_rejection_binding")
                && line.contains("ptr %native_descriptor")
        })
        .expect(&ir);
    assert!(
        lines.iter().any(|line| {
            line.contains("store ptr %rethrow_native_text,")
                && line.contains("ptr @__thaw_pending_exception_native_text")
        }),
        "{ir}"
    );
    assert!(
        lines.iter().any(|line| {
            line.contains("store ptr %rethrow_native_exception,")
                && line.contains("ptr @__thaw_pending_exception_native,")
        }),
        "{ir}"
    );
    assert!(native_text_select.contains("ptr %trusted_native_text"), "{ir}");
    assert!(native_descriptor_select.contains("ptr %native_descriptor"), "{ir}");
}
