// Contracts for lazily promoted captures inside `try`/`catch` and for stack slots emitted inside loops.
// Expected output recorded from node 22.

#[test]
fn bugfix_contract_catch_capture_does_not_alias_later_comparison() {
    let source = r##"function main(): void {
  const a = "x"; const b = "y";
  try {} catch (e: any) { console.log(e === b); }
  console.log(a === b);
}
"##;
    assert_eq!(compile_and_run(source, "bugfix_catch_capture_compare"), "false\n");
}

#[test]
fn bugfix_contract_catch_closure_capture_is_initialised_when_catch_is_skipped() {
    let source = r##"function main(): void {
  const a = "x"; const b = "y";
  try {} catch (e: any) { const f = () => b; console.log(f()); }
  console.log(a === b, b);
  const c = "p"; const d = "q";
  try { console.log((() => d)()); } catch (e: any) { console.log("no"); }
  console.log(c === d, d);
  const g = "m"; const h = "n";
  try { throw new Error("z"); } catch (e: any) { const f = () => h; console.log(f()); }
  console.log(g === h, h);
}
"##;
    assert_eq!(
        compile_and_run(source, "bugfix_catch_closure_capture"),
        "false y\nq\nfalse q\nn\nfalse n\n"
    );
}

#[test]
fn bugfix_contract_loop_try_does_not_grow_the_native_stack() {
    // Each iteration used to allocate fresh catch slots on the native stack.
    let source = r##"function main(): void {
  let n = 0;
  for (let i = 0; i < 3000000; i++) {
    try { n += i % 3; } catch (e: any) { n = -1; }
  }
  console.log(n);
}
"##;
    assert_eq!(compile_and_run(source, "bugfix_loop_try_stack"), "3000000\n");
}

#[test]
fn bugfix_contract_no_fixed_size_alloca_outside_the_entry_block() {
    let source = r##"function main(): void {
  let n = 0;
  for (let i = 0; i < 10; i++) {
    try { n += i; } catch (e: any) { n = -1; }
  }
  console.log(n);
}
"##;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "bugfix_alloca_entry_only");
    compiler.compile_program(&program).unwrap();
    for function in compiler.module.get_functions() {
        for block in function.get_basic_blocks().into_iter().skip(1) {
            let mut next = block.get_first_instruction();
            while let Some(instruction) = next {
                assert_ne!(
                    instruction.get_opcode(),
                    inkwell::values::InstructionOpcode::Alloca,
                    "alloca outside entry in {:?}",
                    function.get_name()
                );
                next = instruction.get_next_instruction();
            }
        }
    }
}
