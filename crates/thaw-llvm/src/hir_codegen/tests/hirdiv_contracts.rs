// `in` on a primitive union member throws a TypeError. Expected output from node 22.

#[test]
fn in_operator_on_primitive_union_member_throws_like_node() {
    let source = r#"
        function probe(v: number | { x: number }): string {
            try { return String("x" in v); } catch (e) { return (e as Error).message; }
        }
        function probe2(v: string | number): string {
            try { return String("x" in v); } catch (e) { return (e as Error).message; }
        }
        function probe3(v: number | { x: number }): string {
            try { return String("y" in v); } catch (e) { return (e as Error).message; }
        }
        function main(): void {
            console.log(probe({ x: 1 }), probe(5), probe2("abc"), probe3({ x: 1 }), probe3(7));
        }
    "#;
    let output = compile_and_run(source, "in_operator_on_primitive_union_member_throws_like_node");
    assert_eq!(
        output,
        "true Cannot use 'in' operator to search for 'x' in 5 Cannot use 'in' operator to search for 'x' in abc false Cannot use 'in' operator to search for 'y' in 7\n"
    );
}

#[test]
fn in_operator_on_local_union_throws_like_node() {
    let source = r#"
        function pick(flag: boolean): number | { x: number } { return flag ? 5 : { x: 1 }; }
        function main(): void {
            const v = pick(true);
            let r = "";
            try { r = String("x" in v); } catch (e) { r = (e as Error).message; }
            const w = pick(false);
            console.log(r, String("x" in w));
        }
    "#;
    let output = compile_and_run(source, "in_operator_on_local_union_throws_like_node");
    assert_eq!(
        output,
        "Cannot use 'in' operator to search for 'x' in 5 true\n"
    );
}
