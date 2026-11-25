// Contracts for object literals whose field bindings compile in place (no closure per field).
// Expected output recorded from node 22.

#[test]
fn objlit_fusion_contract_fields_evaluate_in_source_order_with_side_effects() {
    let source = r##"function main(): void {
  const log: string[] = [];
  function t(name: string, v: number): number { log.push(name); return v; }
  let i = 3;
  const p = { a: t("a", i), b: t("b", i + 1), c: t("c", i * 2) };
  i = 10;
  console.log(log.join(","), p.a, p.b, p.c);
  const q = { x: i++, y: i++, z: i };
  console.log(q.x, q.y, q.z, i);
  const s = { ...{ m: 1, n: 2 }, n: t("n", 5), o: t("o", 6) };
  console.log(s.m, s.n, s.o, log.join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "objlit_fusion_order"),
        "a,b,c 3 4 6\n10 11 12 12\n1 5 6 a,b,c,n,o\n"
    );
}

#[test]
fn objlit_fusion_contract_literals_in_a_loop_do_not_alias_and_closures_see_the_variable() {
    let source = r##"function main(): void {
  const arr: { x: number; y: number }[] = [];
  let k = 0;
  for (let i = 0; i < 4; i++) { arr.push({ x: i, y: k++ }); }
  const f = { v: k, get: () => k, get dbl() { return k * 2; } };
  k = 100;
  console.log(arr.map((e) => e.x + ":" + e.y).join(" "), f.v, f.get(), f.dbl);
}
"##;
    assert_eq!(
        compile_and_run(source, "objlit_fusion_loop"),
        "0:0 1:1 2:2 3:3 4 100 200\n"
    );
}

#[test]
fn objlit_fusion_contract_plain_literal_allocates_without_a_closure() {
    let source = r##"function litloop(): number {
  let t = 0;
  for (let i = 0; i < 10; i++) { const p = { x: i, y: i * 2 }; t += p.x + p.y; }
  return t;
}
function main(): void { console.log(litloop()); }
"##;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "objlit_fusion_ir");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let main = ir.split("define ").find(|f| f.contains("@litloop(")).expect("litloop");
    let main = main.split("\n}\n").next().unwrap();
    assert_eq!(main.matches("call ptr @thaw_arena_alloc").count(), 1, "{main}");
}
