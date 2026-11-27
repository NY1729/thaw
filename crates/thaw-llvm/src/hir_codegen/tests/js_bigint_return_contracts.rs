// A BigInt returned by a thaw-js function reaches an `any` as a bigint (outputs recorded from node 22).

#[test]
fn js_bigint_return_reaches_any() {
    let source = r##"function main(): void {
    const g: any = globalThis;
    const r: any = g.BigInt("5");
    console.log(typeof r);
    const big: any = g.BigInt("12345678901234567890");
    console.log(typeof big, String(big), r + 1n);
    const s: any = g.BigInt.asUintN(8, 300n);
    console.log(typeof s, s);
    const h: bigint = process.hrtime.bigint();
    const h2: bigint = process.hrtime.bigint();
    console.log(h2 >= h, typeof (h2 - h));
}
"##;
    assert_eq!(
        compile_and_run(source, "js_bigint_return_reaches_any"),
        "bigint\nbigint 12345678901234567890 6n\nbigint 44n\ntrue bigint\n"
    );
}
