// Wave JA: arrays/objects returned by methods of `any` values stay live views of their owner.
// Expected output from node 22.

#[test]
fn ja_contract_native_class_method_results_alias_their_owner() {
    let source = r##"class Item { n = 1; bump(): number { this.n += 1; return this.n; } }
class Bag {
  items: Item[] = [new Item()];
  names: string[] = ["a"];
  main: Item = new Item();
  list(): Item[] { return this.items; }
  strs(): string[] { return this.names; }
  first(): Item { return this.main; }
  copy(): number[] { return [1, 2]; }
}
class Shell { bag: Bag = new Bag(); }
async function main(): Promise<void> {
  const v: any = new Shell();
  v.bag.strs().push("b");
  console.log(v.bag.names.length, v.bag.strs().length, v.bag.strs() === v.bag.strs());
  console.log(JSON.stringify(v.bag.strs()), v.bag.strs());
  v.bag.list()[0].bump();
  console.log(v.bag.items[0].n);
  v.bag.first().bump();
  console.log(v.bag.main.n, v.bag.first() === v.bag.main);
  const c = v.bag.copy();
  c.push(3);
  console.log(c.length, v.bag.copy().length);
  for (let i = 0; i < 3; i++) { v.bag.strs().push("x" + i); }
  console.log(v.bag.names.length);
}
"##;
    assert_eq!(compile_and_run(source, "ja_contract_native_class_method_results_alias_their_owner"), "2 2 true\n[\"a\",\"b\"] [ 'a', 'b' ]\n2\n2 true\n3 2\n5\n");
}

#[test]
fn ja_contract_js_owned_method_results_alias_their_owner() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { arr: [1, 2], get() { return this.arr; } };
  const a = o.arr;
  a.push(5);
  console.log("prop", o.arr.length, a.length, a === o.arr);
  const r = o.get();
  r.push(4);
  console.log("meth", o.arr.length, r.length, r === o.arr);
  const f = o.get;
  console.log(typeof f);
}
"##;
    assert_eq!(compile_and_run(source, "ja_contract_js_owned_method_results_alias_their_owner"), "prop 3 3 true\nmeth 4 4 true\nfunction\n");
}

#[test]
fn ja_contract_method_result_chain_and_identity_on_object_literals() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { arr: [1, 2], get() { return this.arr; } };
  const r = o.get();
  console.log(typeof r, Array.isArray(r), r.length);
  r.push(4);
  console.log(o.arr.length, r.length, r === o.arr);
  const q = o.get().length;
  console.log(q);
  o.get().push(9);
  console.log(o.arr.length);
}
"##;
    assert_eq!(compile_and_run(source, "ja_contract_method_result_chain_and_identity_on_object_literals"), "object true 2\n3 3 true\n3\n4\n");
}

#[test]
fn ja_contract_live_method_results_survive_turns_and_nesting() {
    let source = r##"class Reg { items: string[] = ["a"]; tag: string = "t"; names(): string[] { return this.items; } label(): string { return this.tag; } fresh(): string[] { return ["z"]; } }
function sleep(ms: number): Promise<void> { return new Promise<void>(resolve => setTimeout(resolve, ms)); }
async function main(): Promise<void> {
  const o: any = { cfg: { x: 1, deep: { arr: [1] } }, list: [1, 2], conf() { return this.cfg; }, deepArr() { return this.cfg.deep.arr; }, lst() { return this.list; }, n() { return 7; }, fresh() { return [9]; } };
  o.conf().x = 5;
  console.log(o.cfg.x, o.conf() === o.cfg);
  o.deepArr().push(2);
  console.log(o.cfg.deep.arr.length);
  console.log(o.n() + 1, o.fresh().length);
  const f = o.fresh(); f.push(1);
  console.log(f.length, o.fresh().length);
  const r: any = new Reg();
  const names = r.names();
  names.push("b");
  await sleep(5);
  names.push("c");
  console.log(r.items.join(","), r.label(), r.fresh().length);
  for (let i = 0; i < 40; i++) { await sleep(1); r.names().push("n" + i); }
  console.log(r.items.length, names.length === r.items.length, JSON.stringify(r.names()).length);
  const l = o.lst();
  l.length = 1;
  l[1] = 8;
  console.log(o.list.join(","), Array.isArray(l), l.map((x: number) => x * 2).join(","));
}
"##;
    assert_eq!(compile_and_run(source, "ja_contract_live_method_results_survive_turns_and_nesting"), "5 true\n2\n8 1\n2 1\na,b,c t 1\n43 true 243\n1,8 true 2,16\n");
}

#[test]
fn ja_contract_class_method_result_array_aliases_the_owner() {
    let source = r##"class Inner { arr: number[] = [1, 2]; get(): number[] { return this.arr; } }
class Holder { inner: Inner = new Inner(); }
async function main(): Promise<void> {
  const v: any = new Holder();
  v.inner.get().push(3);
  console.log(v.inner.arr.length, v.inner.get() === v.inner.arr);
  const r = v.inner.get();
  r.push(4);
  console.log(v.inner.arr.length, r.length, v.inner.arr[3]);
}
"##;
    assert_eq!(
        compile_and_run(source, "ja_contract_class_method_result_array_aliases_the_owner"),
        "3 true\n4 4 4\n"
    );
}
