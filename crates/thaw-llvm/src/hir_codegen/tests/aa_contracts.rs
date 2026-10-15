// Wave AA: Array.prototype methods on an `any` (Json) array receiver. Expected output recorded from node 22.
// Known limitation: `const a: any = nativeArray` converts to an independent Json copy, so mutating `a` does
// not change `nativeArray` (a layout change would be needed for true aliasing).

#[test]
fn aa_contract_mutators_on_any_array_write_back() {
    let source = r##"async function main(): Promise<void> {
  const j: any = JSON.parse('[3,1,2]');
  const alias = j;
  console.log(j.push(9), alias.length, alias[3]);
  console.log(j.pop(), j.shift(), j.unshift(0), alias.join(","));
  console.log(j.splice(1, 1).length, alias.join(","));
  j.sort();
  console.log(alias.join(","), j.reverse() === alias, alias.join(","));
  j.fill(7);
  console.log(alias.join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "aa_contract_mutators_on_any_array_write_back"),
        "4 4 9\n9 3 3 0,1,2\n1 0,2\n0,2 true 2,0\n7,7\n"
    );
}

#[test]
fn aa_contract_transforms_on_any_array() {
    let source = r##"async function main(): Promise<void> {
  const j: any = JSON.parse('[3,1,2]');
  console.log(j.map((x: number) => x * 2).join(","), j.filter((x: number) => x > 1).length);
  console.log(j.reduce((s: number, x: number) => s + x, 0), j.some((x: number) => x > 2), j.every((x: number) => x > 2));
  console.log(j.find((x: number) => x > 1), j.findIndex((x: number) => x > 1));
  console.log(j.flatMap((x: number) => [x, x]).length, j.at(-1), j.slice(1).join(","), j.concat([4]).length);
  console.log(j.toSorted().join(","), j.toReversed().join(","), j.length);
  let sum = 0;
  j.forEach((x: number) => { sum += x; });
  console.log(sum);
}
"##;
    assert_eq!(
        compile_and_run(source, "aa_contract_transforms_on_any_array"),
        "6,2,4 2\n6 true false\n3 0\n6 2 1,2 4\n1,2,3 2,1,3 3\n6\n"
    );
}

#[test]
fn aa_contract_nested_any_array_push_is_visible_through_parent() {
    let source = r##"async function main(): Promise<void> {
  const k: any = JSON.parse('{"list":[1,2]}');
  k.list.push(3);
  console.log(k.list.length, k.list.reduce((s: number, x: number) => s + x, 0), JSON.stringify(k));
}
"##;
    assert_eq!(
        compile_and_run(source, "aa_contract_nested_any_array_push"),
        "3 6 {\"list\":[1,2,3]}\n"
    );
}

#[test]
fn aa_contract_string_and_map_receivers_in_any_keep_their_methods() {
    let source = r##"async function main(): Promise<void> {
  const s: any = JSON.parse('"hello"');
  console.log(s.slice(1));
  const m: any = new Map([["a", 1]]);
  m.forEach((v: any, k: any) => console.log(k, v));
}
"##;
    assert_eq!(
        compile_and_run(source, "aa_contract_string_and_map_receivers_in_any"),
        "ello\na 1\n"
    );
}
