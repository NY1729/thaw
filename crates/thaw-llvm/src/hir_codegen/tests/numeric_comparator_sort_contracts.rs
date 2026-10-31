// `(x, y) => x - y` / `y - x` over a number[] lowers to a native numeric sort; NaN falls back to
// the generic path (whose NaN order differs from V8; only no-crash is pinned). Expected output is from node 22.

#[test]
fn numeric_comparator_sort_native_asc_desc_and_nan_fallback() {
    let source = r#"
function main(): void {
  const a: number[] = [3, -0, 0, Infinity, -Infinity, 2, 2, Infinity, -5];
  a.sort((x, y) => x - y);
  console.log(a.map((v) => Object.is(v, -0) ? "-0" : String(v)).join(","));
  const d = a.toSorted((p, q) => q - p);
  console.log(d.map((v) => Object.is(v, -0) ? "-0" : String(v)).join(","));
  const n: number[] = [5, 1, NaN, 4, 2];
  n.sort((x, y) => x - y);
  console.log(n.length, n.includes(NaN), n.indexOf(5) >= 0);
}
"#;
    assert_eq!(
        compile_and_run(source, "numeric_comparator_sort_native"),
        "-Infinity,-5,-0,0,2,2,3,Infinity,Infinity\nInfinity,Infinity,3,2,2,-0,0,-5,-Infinity\n5 true true\n"
    );
}
