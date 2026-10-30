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
