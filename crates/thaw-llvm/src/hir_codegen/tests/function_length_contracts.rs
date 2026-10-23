// `f.length` on a declared function is the count of leading parameters before the first
// default-valued or rest parameter. Expected output from node 22 (the `?` marker is JS-erased).

#[test]
fn function_length_contract_declared_function_lengths() {
    let source = r#"function f(a: number, b: number): void {}
function h(a: number = 1, b: number): void {}
function r(a: number, ...rest: number[]): void {}
function o(a?: number): void {}
function z(...r: number[]): void {}
function main(): void {
  console.log(f.length, h.length, r.length, o.length, z.length);
}
"#;
    assert_eq!(
        compile_and_run(source, "function_length_contract_declared_function_lengths"),
        "2 0 1 1 0\n"
    );
}
