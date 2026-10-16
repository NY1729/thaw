// Wave RM: remaining `any` limitations. Expected output recorded from node 22.

#[test]
fn rm_contract_method_call_on_indexed_any_receiver() {
    let source = r####"class C { n = 1; bump(): number { this.n += 1; return this.n; } m() { return 1; } }
async function main(): Promise<void> {
  const arr: number[][] = [[1], [2]];
  const l: any = arr;
  l[0].push(5);
  console.log(JSON.stringify(arr));
  const cs: C[] = [new C()];
  const lc: any = cs;
  console.log(lc[0].bump(), cs[0].n, lc[0].m());
  const o: any = { list: [1, 2] };
  o.list.push(3);
  console.log(o.list.length, JSON.stringify(o));
}
"####;
    assert_eq!(compile_and_run(source, "rm_contract_method_call_on_indexed_any_receiver"), "[[1,5],[2]]\n2 2 1\n3 {\"list\":[1,2,3]}\n");
}

#[test]
fn rm_contract_spread_of_an_any_array() {
    let source = r####"async function main(): Promise<void> {
  const a: any = [1, 2, 3];
  const sp = [...a];
  console.log(sp.length, sp[2]);
  const nums: number[] = [4, 5];
  const an: any = nums;
  const sp2 = [0, ...an];
  console.log(sp2.length, sp2.join(","));
}
"####;
    assert_eq!(compile_and_run(source, "rm_contract_spread_of_an_any_array"), "3 3\n3 0,4,5\n");
}

#[test]
fn rm_contract_async_class_assignable_to_promise_returning_interface() {
    let source = r####"class R { async run(): Promise<number> { return 7; } async x(a: number): Promise<string> { return "x" + a; } }
interface Runner { run(): Promise<number>; x(a: number): Promise<string>; }
async function main(): Promise<void> {
  const r: Runner = new R();
  console.log(await r.run(), await r.x(2));
}
"####;
    assert_eq!(compile_and_run(source, "rm_contract_async_class_assignable_to_promise_returning_interface"), "7 x2\n");
}

#[test]
fn rm_contract_has_own_property_on_class_instance_in_any() {
    let source = r####"class C { n = 1; bump(): number { this.n += 1; return this.n; } m() { return 1; } }
async function main(): Promise<void> {
  const c: any = new C();
  console.log(c.hasOwnProperty("n"), c.hasOwnProperty("bump"), c.hasOwnProperty("m"));
}
"####;
    assert_eq!(compile_and_run(source, "rm_contract_has_own_property_on_class_instance_in_any"), "true false false\n");
}

#[test]
fn rm_contract_non_primitive_element_writes_through_an_array_view() {
    let source = r####"async function main(): Promise<void> {
  const grid: number[][] = [[1], [2]];
  const g: any = grid;
  g[0] = [9, 9];
  g.push([3]);
  console.log(JSON.stringify(grid), grid.length);
  const objs: { a: number }[] = [{ a: 1 }];
  const go: any = objs;
  go[0] = { a: 5 };
  go[1] = { a: 6 };
  console.log(objs[0].a, objs[1].a, objs.length);
  const pairs: [string, number][] = [["x", 1]];
  const gp: any = pairs;
  gp[0] = ["y", 2];
  console.log(pairs[0][0], pairs[0][1]);
}
"####;
    assert_eq!(compile_and_run(source, "rm_contract_non_primitive_element_writes_through_an_array_view"), "[[9,9],[2],[3]] 3\n5 6 2\ny 2\n");
}

#[test]
fn rm_contract_cooked_undefined_through_an_any_tag_parameter() {
    let source = r####"function tag(s: any, ...v: any[]): string {
  return typeof s[0] + "|" + String(s[0] === undefined) + "|" + String(s[1]) + "|" + s.length + "|" + Array.isArray(s) + "|" + JSON.stringify(s);
}
async function main(): Promise<void> {
  console.log(tag`\unicode ${1} ok`);
}
"####;
    assert_eq!(compile_and_run(source, "rm_contract_cooked_undefined_through_an_any_tag_parameter"), "undefined|true| ok|2|true|[null,\" ok\"]\n");
}
