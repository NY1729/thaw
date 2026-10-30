// Contracts for audit2 group C (error/async). Expected text recorded from Node v22.

#[test]
fn error_annotated_callback_param_receives_real_error() {
    let source = r####"function main(): void {
  const ac = new AbortController();
  ac.signal.addEventListener("abort", () => { const r = ac.signal.reason as Error; console.log("abort", r.name, r instanceof Error); });
  ac.abort();
}
"####;
    assert_eq!(
        compile_and_run(source, "error_annotated_callback_param_receives_real_error"),
        "abort AbortError true\n"
    );
}

#[test]
fn unhandled_rejection_handler_with_promise_param_runs() {
    let source = r####"function main(): void {
  process.on("unhandledRejection", (r: unknown, p: Promise<unknown>) => { console.log("UR", r instanceof Error ? r.message : r, p instanceof Promise); });
  Promise.reject(new Error("one"));
  Promise.reject("two");
  const p = Promise.reject(new Error("handled later"));
  setTimeout(() => p.catch(() => console.log("late catch")), 5);
}
"####;
    assert_eq!(
        compile_and_run(source, "unhandled_rejection_handler_with_promise_param_runs"),
        "UR one true\nUR two true\nUR handled later true\nlate catch\n"
    );
}
