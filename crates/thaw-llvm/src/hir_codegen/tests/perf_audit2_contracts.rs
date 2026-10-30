// Audit-2 performance rows: native `new Uint8Array(..)`, in-place immediately-invoked
// argument bindings, and ASCII string fast paths. Expected output is from node 22.

fn compile_to_ir_and_quickjs_use(source: &str, module_name: &str) -> (String, bool) {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, module_name);
    compiler.compile_program(&program).unwrap();
    (compiler.print_to_string(), compiler.uses_quickjs())
}

fn function_ir<'a>(ir: &'a str, name: &str) -> &'a str {
    let start = ir.find(&format!("define internal void @{name}(")).expect("function in IR");
    let end = ir[start..].find("\n}").map(|offset| start + offset).expect("function end");
    &ir[start..end]
}

#[test]
fn new_uint8array_is_a_native_store_without_the_js_realm() {
    let source = r#"
function main(): void {
  const s = new Uint8Array(1000);
  for (let i = 0; i < s.length; i++) s[i] = i;
  let total = 0;
  for (let i = 0; i < s.length; i++) total += s[i];
  const t = new Uint8Array(3.9);
  const f = Uint8Array.from([1]);
  console.log(total, f instanceof Uint8Array, s.length, t.length);
}
"#;
    let (_, uses_quickjs) = compile_to_ir_and_quickjs_use(source, "perf_audit2_u8_native");
    assert!(!uses_quickjs, "new Uint8Array(n) must not need the JS realm");
    assert_eq!(compile_and_run(source, "perf_audit2_u8_native_run"), "124716 true 1000 3\n");
}

#[test]
fn new_uint8array_rejects_a_negative_length_like_node() {
    let source = r#"
function main(): void {
  try { const bad = new Uint8Array(-1); console.log(bad.length); } catch (e: any) { console.log(e.name, e.message); }
  const nan = new Uint8Array(NaN);
  const zero = new Uint8Array(0);
  console.log(nan.length, zero.length);
}
"#;
    assert_eq!(
        compile_and_run(source, "perf_audit2_u8_negative"),
        "RangeError Invalid typed array length: -1\n0 0\n"
    );
}

#[test]
fn immediately_invoked_native_argument_bindings_allocate_no_closures() {
    let source = r#"
function main(): void {
  const s = "item123456";
  let h = 0;
  for (let k = 0; k < s.length; k++) h = (Math.imul(h, 31) + s.charCodeAt(k)) | 0;
  console.log(h);
}
"#;
    let (ir, _) = compile_to_ir_and_quickjs_use(source, "perf_audit2_iife");
    let body = function_ir(&ir, "thaw_user_main");
    assert!(!body.contains("closure_alloc"), "bindings must be compiled in place: {body}");
    assert!(!body.contains("closure_code"), "no indirect closure call expected: {body}");
    assert_eq!(compile_and_run(source, "perf_audit2_iife_run"), "964011830\n");
}

#[test]
fn ascii_and_non_ascii_string_operations_match_node() {
    let source = r#"
function main(): void {
  const a = "hello world";
  const u = "héllo 😀 wörld";
  for (const s of [a, u, ""]) {
    console.log(s.length, s.charCodeAt(0), s.charCodeAt(4), s.charCodeAt(s.length - 1), s.charCodeAt(99), s.charCodeAt(-1));
    console.log(s.slice(1, 4), s.slice(-3), s.slice(3, 1), s.substring(4, 1), s.substring(-2, 3), s.substr(-4, 2), s.substr(2));
  }
  console.log("a" < "b", "z" < "é", "😀" < "￿", "￿" < "😀", "abc" < "abd", "abc" === "abc", "ab" < "abc", "abc" > "ab");
  console.log(["b", "a", "é", "😀", "￿", "B", ""].sort().join("|"));
}
"#;
    assert_eq!(
        compile_and_run(source, "perf_audit2_strings"),
        "11 104 111 100 NaN NaN\nell rld  ell hel or llo world\n\
14 104 111 100 NaN NaN\n\u{e9}ll rld  \u{e9}ll h\u{e9}l \u{f6}r llo \u{1f600} w\u{f6}rld\n\
0 NaN NaN NaN NaN NaN\n      \ntrue true true false true true true true\n|B|a|b|\u{e9}|\u{1f600}|\u{ffff}\n"
    );
}
