// `for...of` over a heterogeneous array literal of closures is typed as a tuple; it iterates
// as an array of the members' common callable type. Expected output recorded from node 22.

#[test]
fn forof_tuple_round8_mixed_closure_returns_iterate_and_call() {
    let source = r#"
function main(): void {
  for (const f of [() => 1, () => "a"]) console.log(typeof f());
}
"#;
    assert_eq!(
        compile_and_run(source, "forof_tuple_round8_mixed_closure_returns_iterate_and_call"),
        "number\nstring\n"
    );
}

#[test]
fn forof_tuple_round8_throwing_closure_is_caught_in_loop_body() {
    let source = r#"
function main(): void {
  for (const f of [() => { throw new Error("one"); }, () => 2]) {
    try { console.log(f()); } catch (e) { console.log("caught", (e as Error).message); }
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "forof_tuple_round8_throwing_closure_is_caught_in_loop_body"),
        "caught one\n2\n"
    );
}
