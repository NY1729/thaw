// `console.log(error)` prints the stack frames, own props, `[cause]` and `[errors]` like Node 22
// (`Name: message`, frame lines, then `{ ... }`). Frame lines are Thaw's (`input.ts` paths); the
// layout around them is real node output.

#[test]
fn error_inspect_prints_stack_props_and_cause() {
    let source = r#"function main(): void {
  console.log(new Error("log me"));
  const e: any = new Error("outer", { cause: new RangeError("inner") });
  e.code = "E_X";
  console.log(e);
  console.log({ e: new Error("nested") }, [new RangeError("in arr")]);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_inspect_prints_stack_props_and_cause"),
        "Error: log me\n    at main (input.ts:1:1)\nError: outer\n    at main (input.ts:1:1) {\n  code: 'E_X',\n  [cause]: RangeError: inner\n      at main (input.ts:1:1)\n}\n{\n  e: Error: nested\n      at main (input.ts:1:1)\n} [\n  RangeError: in arr\n      at main (input.ts:1:1)\n]\n"
    );
}
