// Callback storage contracts: `any`-typed comparator/predicate callbacks on native arrays. Callbacks that only
// read their parameters keep the receiver native (fast path); callbacks that mutate a parameter object keep the
// receiver dynamic so the mutation stays visible through aliases. Expected output from node 22.

#[test]
fn fx18_any_comparator_sort_20000_numbers_is_fast_and_correct() {
    let source = r#"
function main(): void {
  const a: number[] = [];
  for (let i = 0; i < 20000; i++) a.push((i * 7919) % 20011);
  a.sort((x: any, y: any) => y - x);
  console.log(a[0], a[19999], a.length);
}
"#;
    let output = compile_and_run(source, "fx18_any_comparator_sort_20000_numbers_is_fast_and_correct");
    assert_eq!(output, "20010 0 20000\n");
}

#[test]
fn fx18_any_comparator_sort_2000_numbers_matches_node() {
    let source = r#"
function main(): void {
  const a: number[] = [];
  for (let i = 0; i < 2000; i++) a.push((i * 7919) % 2003);
  a.sort((x: any, y: any) => y - x);
  console.log(a[0], a[1999], a.length);
  console.log(a.slice(0, 5).join(","), a.slice(1995).join(","));
}
"#;
    let output = compile_and_run(source, "fx18_any_comparator_sort_2000_numbers_matches_node");
    assert_eq!(output, "2002 0 2000\n2002,2001,2000,1999,1998 4,3,2,1,0\n");
}

#[test]
fn fx18_callbacks_that_mutate_params_keep_aliases_in_sync() {
    let source = r#"
function main(): void {
  const objs = [{ v: 3 }, { v: 1 }, { v: 2 }];
  const first = objs[0];
  objs.sort((x: any, y: any) => { x.seen = 1; y.seen = 1; return x.v - y.v; });
  console.log(objs.map((o: any) => o.v).join(","), (first as any).seen);
  const o2 = [{ v: 3 }, { v: 1 }];
  const keep = o2[0];
  o2.forEach((o: any) => { o.v = o.v * 10; });
  console.log(keep.v, o2[1].v);
  const m = [{ v: 5 }, { v: 4 }];
  const k = m[0];
  const r = m.filter((o: any) => { o.v++; return o.v > 5; });
  console.log(k.v, m[1].v, r.length);
  const nums = [3, 1, 2];
  nums.sort((x: any, y: any) => y - x);
  console.log(nums.join(","), nums.some((x: any) => x > 2), nums.every((x: any) => x > 0), nums.find((x: any) => x < 3), nums.filter((x: any) => x % 2 == 1).join(","));
}
"#;
    let output = compile_and_run(source, "fx18_callbacks_that_mutate_params_keep_aliases_in_sync");
    assert_eq!(output, "1,2,3 1\n30 10\n6 5 1\n3,2,1 true true 2 3,1\n");
}
