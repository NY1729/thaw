// `string | null` (and `string | undefined`, `string | number`) operands compared with a
// string literal or a string-typed value, in both operand orders, for ===, !==, == and !=.
// Expected output recorded from node 22.23.1 running the same logic.

#[test]
fn union_with_string_member_strict_and_loose_equality_matches_node() {
    let source = r#"
        function pick(): string | null { return '/a/b'; }
        function pickNull(): string | null { return null; }
        function pickU(): string | undefined { return '/a/b'; }
        function pickUNull(): string | undefined { return undefined; }
        function pickN(): string | number { return '/a/b'; }
        function pickNum(): string | number { return 7; }
        function pickNumStr(): string | number { return 'x'; }
        function main(): void {
            console.log(pick() === '/a/b', pick() === '/x', pickNull() === '/a/b', pick() !== '/a/b', pickNull() !== '/a/b');
            console.log('/a/b' === pick(), '/x' === pick(), '/a/b' !== pick(), null === pickNull(), pickNull() === null);
            console.log(pickU() === '/a/b', pickU() === '/x', pickUNull() === '/a/b', pickUNull() !== '/a/b', '/a/b' === pickU());
            console.log(pickN() === '/a/b', pickN() === 'x', pickN() === 7, pickNum() === 7, pickNum() === '7', 7 === pickNum(), '/a/b' === pickN());
            console.log(pickN() !== 7, pickNum() !== 7, pickN() !== '/a/b', 7 !== pickN());
            let v: string | null = pick();
            console.log(v === '/a/b', v !== '/a/b');
            v = pickNull();
            console.log(v === '/a/b', v === null, v !== '/a/b');
            let w: string | null = pick();
            console.log(w === '/a/b', '/a/b' === w, w !== '/x');
            console.log(pick() == '/a/b', pick() != '/a/b', '/a/b' == pick(), pickNull() == '/a/b', pickNull() != '/a/b', pickU() == '/a/b', pickN() == '/a/b', pickN() == 7, pickNum() == 7, pickNum() == '7', 7 == pickNum(), pickN() != 'x');
            console.log(pickNull() == null, pickUNull() == null, pick() == null, pickUNull() === undefined, pickNumStr() === 'x');
            let s: string = '/a/b';
            console.log(pick() === s, s === pick(), pick() !== s, pickN() === s, pickU() === pickN(), pick() === pickNull());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_string_strict_and_loose_equality"),
        "true false false false true\n\
         true false false true true\n\
         true false false true true\n\
         true false false true false true true\n\
         true false false true\n\
         true false\n\
         false true true\n\
         true true true\n\
         true false true false true true true false true true true true\n\
         true true false true true\n\
         true true false true true false\n"
    );
}

// Wrapper shapes (`string | null | undefined`, `number | null`), the same union on both sides, and
// mixed union pairs, through ===, !==, == and != in both operand orders.
// Expected output recorded from node 22.23.1 running the same logic.

#[test]
fn union_wrapper_and_union_pair_equality_matches_node() {
    let source = r#"
        function nstr(x: string | null): string | null { return x; }
        function ustr(x: string | undefined): string | undefined { return x; }
        function nnum(x: number | null): number | null { return x; }
        function main(): void {
            let a: string | null = nstr('/a/b');
            let n: string | null = nstr(null);
            let u: string | undefined = ustr(undefined);
            let nu: string | null = nstr('/a/b');
            let nun: string | null = nstr(null);
            let num: number | null = nnum(7);
            console.log(nu === '/a/b', nun === '/a/b', nun !== '/a/b', nu !== '/a/b', '/a/b' === nu);
            console.log(nu == '/a/b', nun == '/a/b', nun != '/a/b', nu != '/a/b', '/a/b' != nu);
            console.log(nu === 'x' ? 1 : 2, a === nu, nu === a, n === nu);
            let x: string | null = nstr(null);
            console.log(x === '/a/b', x == '/a/b', x == null, x === null, x === undefined);
            let y: string | undefined = ustr(undefined);
            console.log(y === '/a/b', y == '/a/b', y == null, y === undefined);
            let z: string | null = nstr('/a/b');
            console.log(z === '/a/b', z == '/a/b', z !== '/a/b', z != '/a/b');
            console.log(num == '7', num === '7', '7' == num, num != '7');
            console.log(nu == num, num == nu, num === nu, nu === num);
            console.log(nu == u, u == nu, nu === u, n == nu, nun == n);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_wrapper_and_union_pair_equality"),
        "true false true false true\n\
         true false true false false\n\
         2 true true false\n\
         false false true true false\n\
         false false true true\n\
         true true false false\n\
         true false true false\n\
         false false false false\n\
         false false false false true\n"
    );
}
