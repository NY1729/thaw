// Real `Error.stack` frames: function names from the compiler-emitted table, innermost first,
// bounded by `Error.stackTraceLimit`. The file part is the build path, so only names are pinned.

#[test]
fn error_stack_lists_the_calling_functions() {
    let source = r#"function inner(): string {
  return new Error("boom").stack!;
}
function middle(): string {
  return inner();
}
function outer(): string {
  return middle();
}
function names(stack: string): string {
  return stack.split("\n").slice(1).map((line: string) => line.trim().split(" (")[0]).join("|");
}
function main(): void {
  console.log(names(outer()));
  Error.stackTraceLimit = 2;
  console.log(names(outer()));
  Error.stackTraceLimit = 0;
  console.log(JSON.stringify(outer().split("\n").length));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_stack_lists_the_calling_functions"),
        "at inner|at middle|at outer|at main\nat inner|at middle\n1\n"
    );
}
