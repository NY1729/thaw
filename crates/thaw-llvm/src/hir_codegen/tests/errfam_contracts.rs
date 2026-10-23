// Error-handling contracts: `e.constructor`, `String(err)`/template/`+` rendering, `Error(..)` without `new`,
// framed errors reaching JS as real Errors, and a catchable stack overflow. Expected output from node 22.

#[test]
fn error_contract_builtin_constructor_name_and_identity() {
    let source = r#"function main(): void {
  try { JSON.parse('{'); } catch (e: any) { console.log(e.constructor.name, e.constructor === SyntaxError, e instanceof SyntaxError); }
  try { throw new RangeError('r'); } catch (e: any) { console.log(e.constructor.name, e.constructor === RangeError, e.constructor === TypeError); }
  const o: any = JSON.parse('null');
  try { o.x; } catch (e: any) { console.log(e.constructor.name, e.constructor === TypeError); }
  try { throw new Error('plain'); } catch (e: any) { console.log(e.constructor.name, e.constructor === Error); }
  const t = new TypeError('t');
  console.log(t.constructor.name, t.constructor === TypeError);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_contract_builtin_constructor_name_and_identity"),
        "SyntaxError true true\nRangeError true false\nTypeError true\nError true\nTypeError true\n"
    );
}

#[test]
fn error_contract_string_conversion_keeps_name_prefix() {
    let source = r#"function main(): void {
  const t = new TypeError('t');
  console.log(String(t), `${t}`, 'x: ' + t, t + '');
  let s = 'a: ';
  s += t;
  console.log(s);
  try { throw new RangeError('r'); } catch (e: any) { console.log(String(e), `${e}`, 'c: ' + e); }
  console.log('plain ' + 'text', `${'plain'}`);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_contract_string_conversion_keeps_name_prefix"),
        "TypeError: t TypeError: t x: TypeError: t TypeError: t\na: TypeError: t\nRangeError: r RangeError: r c: RangeError: r\nplain text plain\n"
    );
}

#[test]
fn error_contract_subclass_string_conversion() {
    let source = r#"class MyErr extends Error { constructor(m: string) { super(m); this.name = 'MyErr'; } }
class Sub extends MyErr {}
class E2 extends Error {}
function main(): void {
  const m = new MyErr('oops');
  console.log(String(m), `${m}`, 'x: ' + m, m.toString());
  console.log(String(new Sub('q')), `${new E2('z')}`);
  try { throw new MyErr('mm'); } catch (e: any) { console.log(String(e), `${e}`, 'c: ' + e); }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_contract_subclass_string_conversion"),
        "MyErr: oops MyErr: oops x: MyErr: oops MyErr: oops\nMyErr: q Error: z\nMyErr: mm MyErr: mm c: MyErr: mm\n"
    );
}

#[test]
fn error_contract_call_without_new_is_an_instance() {
    let source = r#"function main(): void {
  const e0 = Error('x');
  console.log(e0 instanceof Error, e0.message, String(e0), e0.name);
  const e1 = TypeError('y');
  console.log(e1 instanceof TypeError, e1 instanceof Error, e1.name, String(e1));
  const a = new Error('x');
  console.log(a instanceof Error, a instanceof TypeError);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_contract_call_without_new_is_an_instance"),
        "true x Error: x Error\ntrue true TypeError TypeError: y\ntrue false\n"
    );
}

#[test]
fn error_contract_stack_overflow_is_a_catchable_range_error() {
    let source = r#"function r(n: number): number { return r(n + 1) + 1; }
function depth(n: number): number { return n === 0 ? 0 : depth(n - 1) + 1; }
function main(): void {
  try { r(0); console.log('no'); } catch (e: any) { console.log('caught', e instanceof RangeError, e.message); }
  console.log(depth(5000));
  try { r(0); } catch (e: any) { console.log(String(e)); }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_contract_stack_overflow_is_a_catchable_range_error"),
        "caught true Maximum call stack size exceeded\n5000\nRangeError: Maximum call stack size exceeded\n"
    );
}

#[test]
fn error_contract_framed_error_argument_reaches_js_as_error() {
    let source = r#"declare function __thaw_typed_js_64416e79(value: any): string;
declare function __thaw_typed_js_64537472(value: string): string;
function main(): void {
  loadScript("globalThis.dAny = v => (v instanceof Error) + '|' + v.name + '|' + v.message + '|' + String(v); globalThis.dStr = globalThis.dAny;");
  const e = new TypeError("e");
  console.log(__thaw_typed_js_64416e79(new TypeError("e")));
  console.log(__thaw_typed_js_64537472(e));
  try { throw new RangeError('r'); } catch (c) { console.log(__thaw_typed_js_64416e79(c)); }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_contract_framed_error_argument_reaches_js_as_error"),
        "true|TypeError|e|TypeError: e\ntrue|TypeError|e|TypeError: e\ntrue|RangeError|r|RangeError: r\n"
    );
}
