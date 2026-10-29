// `Error.stack` frames carry the function's definition position: `at fn (file:line:col)`.

#[test]
fn error_stack_frames_carry_definition_line_and_column() {
    let source = r#"function inner(): string {
  return new Error("boom").stack!;
}
class K {
  m(): string {
    return inner();
  }
}
function places(stack: string): string {
  return stack.split("\n").slice(1).map((line: string) => {
    const parts = line.trim().split(":");
    return parts[0].split(" ")[1] + "@" + parts[parts.length - 2] + ":" + parts[parts.length - 1].replace(")", "");
  }).join("|");
}
function main(): void {
  console.log(places(new K().m()));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_stack_frames_carry_definition_line_and_column"),
        "inner@1:1|K.m@5:3|main@15:1\n"
    );
}

#[test]
fn runtime_thrown_errors_keep_real_frames() {
    let source = r#"function fail(): string {
  try {
    const o: any = null;
    o.x.y;
  } catch (e: any) {
    return e.stack;
  }
  return "";
}
function main(): void {
  console.log(fail().split("\n")[1].includes("at fail"));
}
"#;
    assert_eq!(compile_and_run(source, "runtime_thrown_errors_keep_real_frames"), "true\n");
}

#[test]
fn capture_stack_trace_on_a_plain_object_lists_frames() {
    let source = r#"function mk(): any {
  const o: any = { name: "X", message: "hi" };
  Error.captureStackTrace(o);
  return o.stack;
}
function wrap(): any { return mk(); }
function main(): void {
  console.log(wrap().split("\n").slice(1).map((l: string) => l.trim().split(" ")[1]).join("|"));
}
"#;
    assert_eq!(compile_and_run(source, "capture_stack_trace_on_a_plain_object_lists_frames"), "mk|wrap|main\n");
}
