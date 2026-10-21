// Wave NB3: generator `.throw(value)` passes the value through unchanged (identity,
// primitives, undefined), and `yield*` still forwards it. Expected output from node 22.

#[test]
fn nb3_contract_generator_throw_keeps_object_identity() {
    let source = r##"function* generator(): Generator<any, void, unknown> {
  try { yield 1; } catch (error) { yield error; }
}
function main(): void {
  const marker: any = { value: 42 };
  const iterator = generator();
  iterator.next();
  const result = iterator.throw(marker);
  console.log(result.value === marker, result.value.value, result.done);
}"##;
    assert_eq!(
        compile_and_run(source, "nb3_contract_generator_throw_keeps_object_identity"),
        "true 42 false\n"
    );
}

#[test]
fn nb3_contract_generator_throw_passes_primitives_and_undefined() {
    let source = r##"function* g(): Generator<any, void, unknown> {
  try { yield 1; } catch (error) { yield error; }
}
function main(): void {
  const it = g();
  it.next();
  console.log(it.throw(7).value);
  const it2 = g();
  it2.next();
  console.log(it2.throw("s").value);
  const it3 = g();
  it3.next();
  console.log(it3.throw(undefined).value);
}"##;
    assert_eq!(
        compile_and_run(source, "nb3_contract_generator_throw_passes_primitives_and_undefined"),
        "7\ns\nundefined\n"
    );
}

#[test]
fn nb3_contract_generator_throw_caught_object_is_not_stringified() {
    let source = r##"function* g(): Generator<number, void, unknown> {
  try { yield 1; } catch (error: any) { console.log("caught", error.code); }
}
function main(): void {
  const e: any = { code: "E42" };
  const it = g();
  it.next();
  console.log(it.throw(e).done);
}"##;
    assert_eq!(
        compile_and_run(source, "nb3_contract_generator_throw_caught_object_is_not_stringified"),
        "caught E42\ntrue\n"
    );
}
