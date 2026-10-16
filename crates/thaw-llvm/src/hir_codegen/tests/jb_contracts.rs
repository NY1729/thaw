// Wave JB: cyclic Json arrays stringify without recursing forever, and an annotated forEach
// callback works on an `any` Map/Set. Expected output from node 22.

#[test]
fn jb_contract_cyclic_json_array_joins_like_node() {
    let source = r##"async function main(): Promise<void> {
  const j: any = JSON.parse("[1,2]");
  j.push(j);
  console.log(String(j));
  console.log(j.join("-"));
  console.log(Number(j), "" + j);
  const nat: number[][] = [[1]];
  const v: any = nat;
  v[0] = j;
  console.log(nat.length, nat[0].length, v[0] === j);
}
"##;
    assert_eq!(
        compile_and_run(source, "jb_contract_cyclic_json_array_joins_like_node"),
        "1,2,\n1-2-\nNaN 1,2,\n1 3 true\n"
    );
}

#[test]
fn jb_contract_annotated_foreach_callback_on_any_map_and_set() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { m: new Map<string, number>(), s: new Set<number>([5, 6]) };
  o.m.set("a", 1);
  o.m.set("b", 2);
  o.m.forEach((v: number, k: string) => console.log(k, v));
  o.s.forEach((v: number) => console.log(v));
  const list: any = [3, 4];
  list.forEach((x: number, i: number) => console.log(i, x));
}
"##;
    assert_eq!(
        compile_and_run(source, "jb_contract_annotated_foreach_callback_on_any_map_and_set"),
        "a 1\nb 2\n5\n6\n0 3\n1 4\n"
    );
}
