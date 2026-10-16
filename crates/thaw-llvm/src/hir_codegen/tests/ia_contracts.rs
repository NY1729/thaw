// Wave IA: native Map/Set held in `any` literals (object fields, array elements, nested) and
// Map/Set methods on host-owned collections. Expected output from node 22.

#[test]
fn ia_contract_map_and_set_fields_of_any_literals_work() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { m: new Map<string, number>(), s: new Set<string>(), n: 1 };
  o.m.set("a", 1).set("b", 2);
  console.log(o.m.size, o.m.has("a"), o.m.has("z"), o.m.get("b"));
  console.log(o.m.delete("a"), o.m.delete("a"), o.m.size);
  o.s.add("x").add("y");
  console.log(o.s.size, o.s.has("x"), o.s.delete("x"), o.s.has("x"));
  console.log(o.m === o.m, [...o.s].length, [...o.m].length);
  console.log(o);
  const seen: any[] = [];
  o.m.forEach((v: any, k: any) => { seen.push(k + v); });
  console.log(seen.join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "ia_contract_map_and_set_fields_of_any_literals_work"),
        "2 true false 2\ntrue false 1\n2 true true false\ntrue 1 1\n{ m: Map(1) { 'b' => 2 }, s: Set(1) { 'y' }, n: 1 }\nb2\n"
    );
}

#[test]
fn ia_contract_map_and_set_elements_of_any_arrays_and_nested_literals() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { n: { m: new Map<string, number>() }, list: [new Set<number>()] };
  o.n.m.set("a", 1);
  o.list[0].add(2);
  console.log(o.n.m.get("a"), o.list[0].has(2), o.list[0].size);
  const list: any = [new Map<string, number>(), new Set<number>()];
  list[0].set("k", 5);
  list[1].add(7);
  console.log(list[0].get("k"), list[1].has(7), list);
  const plain: any = { get(k: string) { return "got " + k; }, has: (k: string) => k === "q" };
  console.log(plain.get("z"), plain.has("q"), plain.has("r"));
}
"##;
    assert_eq!(
        compile_and_run(source, "ia_contract_map_and_set_elements_of_any_arrays_and_nested_literals"),
        "1 true 1\n5 true [ Map(1) { 'k' => 5 }, Set(1) { 7 } ]\ngot z true false\n"
    );
}
