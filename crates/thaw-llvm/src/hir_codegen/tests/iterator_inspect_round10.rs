// console.log of generator objects and native iterator objects. Expected output from node 22.
// A plain function keeps its `[Function: name]` label.

#[test]
fn iterator_inspect_round10_generator_objects() {
    let source = r#"
        function* g(): Generator<number> {
            yield 1;
        }
        function main(): void {
            const it = g();
            console.log(it, g());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "iterator_inspect_round10_generator_objects"),
        "Object [Generator] {} Object [Generator] {}\n"
    );
}

#[test]
fn iterator_inspect_round10_array_and_string_iterators() {
    let source = r#"
        function main(): void {
            console.log([1][Symbol.iterator](), "x"[Symbol.iterator]());
            console.log(Object.prototype.toString.call("x"[Symbol.iterator]()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "iterator_inspect_round10_array_string"),
        "Object [Array Iterator] {} Object [String Iterator] {}\n[object String Iterator]\n"
    );
}

#[test]
fn iterator_inspect_round10_plain_function_keeps_label() {
    let source = r#"
        function f(): void {}
        function main(): void {
            console.log(f);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "iterator_inspect_round10_plain_function"),
        "[Function: f]\n"
    );
}
