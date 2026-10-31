// Array sort/toSorted: NaN comparator results count as 0 and stable TimSort-equivalent output; expected output from node 22.

#[test]
fn sort_nan_comparator_contract_matches_node() {
    let source = r##"function main(): void {
  console.log([5, 1, NaN, 4, 2].sort((x, y) => x - y).join(","));
  console.log([3, NaN, 1].toSorted((x, y) => x - y).join(","));
}
"##;
    assert_eq!(compile_and_run(source, "sort_nan_comparator_contract"), "1,2,4,5,NaN\n3,NaN,1\n");
}

#[test]
fn sort_random_arrays_contract_matches_node() {
    let source = r##"let seed = 12345;
function rnd(m: number): number { seed = (seed * 1103515245 + 12345) % 2147483648; return seed % m; }
function main(): void {
  console.log([5, 1, NaN, 4, 2].sort((x, y) => x - y).join(","));
  console.log([3, NaN, 1].toSorted((x, y) => x - y).join(","));
  let h1 = 0, h2 = 0, h3 = 0;
  for (let n = 1; n <= 300; n++) {
    const objs: { k: number; i: number }[] = [];
    const nums: number[] = [];
    for (let i = 0; i < n; i++) { const v = rnd(n < 20 ? 5 : 40); objs.push({ k: v, i }); nums.push(v * 3 - 20); }
    const so = objs.sort((a, b) => a.k - b.k);
    for (let i = 0; i < n; i++) h1 = (h1 * 31 + so[i].k * 1000 + so[i].i) % 1000000007;
    const sn = nums.toSorted((a, b) => a - b);
    for (let i = 0; i < n; i++) h2 = (h2 * 31 + sn[i] + 100) % 1000000007;
    const strs = nums.map((v) => "s" + (v % 7));
    const ss = strs.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    for (let i = 0; i < n; i++) h3 = (h3 * 31 + ss[i].charCodeAt(1)) % 1000000007;
  }
  console.log(h1, h2, h3);
}
"##;
    assert_eq!(
        compile_and_run(source, "sort_random_arrays_contract"),
        "1,2,4,5,NaN\n3,NaN,1\n556655060 189275089 985022790\n"
    );
}
