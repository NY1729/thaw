// Object.prototype.toString tags for native iterators, typed arrays and WeakRef.
// Expected output from node 22. A plain function must keep `[object Function]`.

#[test]
fn tostring_tags_round8_array_map_set_iterators() {
    let source = r#"
        function f(): void {}
        function main(): void {
            const xs: number[] = [1, 2];
            const m = new Map<string, number>();
            const s = new Set<number>();
            console.log(Object.prototype.toString.call(xs.values()));
            console.log(Object.prototype.toString.call(m.entries()));
            console.log(Object.prototype.toString.call(s.values()));
            console.log(Object.prototype.toString.call(f));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tostring_tags_round8_iterators"),
        "[object Array Iterator]\n[object Map Iterator]\n[object Set Iterator]\n[object Function]\n"
    );
}

#[test]
fn tostring_tags_round8_typed_array() {
    let source = r#"
        function main(): void {
            const bytes = new Uint8Array(1);
            console.log(Object.prototype.toString.call(bytes));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tostring_tags_round8_typed_array"),
        "[object Uint8Array]\n"
    );
}

#[test]
fn tostring_tags_round8_generator_function_and_object() {
    let source = r#"
        function* gen(): Generator<number> {
            yield 1;
        }
        function main(): void {
            console.log(Object.prototype.toString.call(gen));
            console.log(Object.prototype.toString.call(gen()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tostring_tags_round8_generator"),
        "[object GeneratorFunction]\n[object Generator]\n"
    );
}

#[test]
fn tostring_tags_round8_weakref() {
    let source = r#"
        function main(): void {
            console.log(Object.prototype.toString.call(new WeakRef<object>({ a: 1 })));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tostring_tags_round8_weakref"),
        "[object WeakRef]\n"
    );
}
