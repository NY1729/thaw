// Wave GC: `any` receivers reached through call results, `?.`, element access, `await`, property
// chains and callbacks. Expected output from node 22.

#[test]
fn gc_contract_call_result_push() {
    let source = r##"function mk(): any { return { a: [1, 2] }; }
async function main(): Promise<void> {
  const o = mk();
  mk().a.push(3);
  o.a.push("x");
  console.log(o.a.length, JSON.stringify(o.a));
  const r = mk();
  r.a.push({ k: 1 });
  console.log(JSON.stringify(r));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_call_result_push"),
        "3 [1,2,\"x\"]\n{\"a\":[1,2,{\"k\":1}]}\n"
    );
}

#[test]
fn gc_contract_global_any_literal_call_chain() {
    let source = r##"let store: any = { a: [1] };
function mk(): any { return store; }
async function main(): Promise<void> {
  mk().a.push(2);
  mk().a.push("s");
  console.log(JSON.stringify(store), mk().a.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_global_any_literal_call_chain"),
        "{\"a\":[1,2,\"s\"]} 3\n"
    );
}

#[test]
fn gc_contract_optional_chain_short_circuits() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { list: [1] };
  o?.list.push(2);
  o.list?.push("z");
  const n: any = null;
  console.log(JSON.stringify(o.list), n?.list.push(1));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_optional_chain_short_circuits"),
        "[1,2,\"z\"] undefined\n"
    );
}

#[test]
fn gc_contract_element_access_push() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { k: [1] };
  o["k"].push(2);
  const key = "k";
  o[key].push("q");
  console.log(JSON.stringify(o.k), o["k"].length);
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_element_access_push"),
        "[1,2,\"q\"] 3\n"
    );
}

#[test]
fn gc_contract_paren_nonnull_as_receivers() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { list: [1] };
  (o.list).push(2);
  o.list!.push(3);
  (o as any).list.push(4);
  ((o as any)).list.push(5);
  console.log(JSON.stringify(o.list));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_paren_nonnull_as_receivers"),
        "[1,2,3,4,5]\n"
    );
}

#[test]
fn gc_contract_await_result_receiver() {
    let source = r##"async function f(): Promise<any> { return { list: [1] }; }
async function main(): Promise<void> {
  (await f()).list.push(2);
  const x = await f();
  x.list.push("s");
  console.log(JSON.stringify(x));
  const y = (await f()).list;
  y.push(7);
  console.log(JSON.stringify(y));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_await_result_receiver"),
        "{\"list\":[1,\"s\"]}\n[1,7]\n"
    );
}

#[test]
fn gc_contract_json_parse_method_chain() {
    let source = r##"async function main(): Promise<void> {
  const s = '{"items":[3,1,2],"obj":{"xs":[5,4]}}';
  console.log(JSON.stringify(JSON.parse(s).items.map((x: any) => x * 2)));
  console.log(JSON.stringify(JSON.parse(s).items.filter((x: any) => x > 1)));
  console.log(JSON.parse(s).items.reduce((a: any, b: any) => a + b, 0));
  console.log(JSON.stringify(JSON.parse(s).items.sort()));
  console.log(JSON.stringify(JSON.parse(s).obj.xs.slice(0, 1)));
  const d = JSON.parse(s);
  d.items.splice(1, 1, "a", "b");
  d.items.fill(0, 0, 1);
  console.log(JSON.stringify(d));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_json_parse_method_chain"),
        "[6,2,4]\n[3,2]\n6\n[1,2,3]\n[5]\n{\"items\":[0,\"a\",\"b\",2],\"obj\":{\"xs\":[5,4]}}\n"
    );
}

#[test]
fn gc_contract_destructured_any() {
    let source = r##"async function main(): Promise<void> {
  const h: any = { list: [1], o: { xs: [1] } };
  const { list } = h;
  list.push(2);
  const { o: { xs } } = h;
  xs.push("t");
  const [first] = [h.list];
  first.push(3);
  console.log(JSON.stringify(h));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_destructured_any"),
        "{\"list\":[1,2,3],\"o\":{\"xs\":[1,\"t\"]}}\n"
    );
}

#[test]
fn gc_contract_forof_and_push_spread_of_any() {
    let source = r##"async function main(): Promise<void> {
  const rows: any = [{ list: [1] }, { list: [2] }];
  for (const r of rows) r.list.push(0);
  const all: any[] = [];
  for (const r of rows) all.push(...r.list);
  console.log(JSON.stringify(rows), JSON.stringify(all));
  const copy = [...rows[0].list];
  console.log(JSON.stringify(copy));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_forof_and_push_spread_of_any"),
        "[{\"list\":[1,0]},{\"list\":[2,0]}] [1,0,2,0]\n[1,0]\n"
    );
}

#[test]
fn gc_contract_class_field_any_push() {
    let source = r##"class Box {
  data: any = { list: [1] };
  add(v: any): void { this.data.list.push(v); }
  get len(): number { return this.data.list.length; }
}
async function main(): Promise<void> {
  const b = new Box();
  b.add(2);
  b.add("s");
  b.data.list.push({ q: 1 });
  console.log(b.len, JSON.stringify(b.data));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_class_field_any_push"),
        "4 {\"list\":[1,2,\"s\",{\"q\":1}]}\n"
    );
}

#[test]
fn gc_contract_method_on_call_result() {
    let source = r##"function mk(): any { return { n: 0, inc() { this.n++; return this.n; }, name: "abc" }; }
async function main(): Promise<void> {
  console.log(mk().inc(), mk().name.toUpperCase(), mk().name.length);
  const o = mk();
  o.inc(); o.inc();
  console.log(o.n, o.name.slice(1));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_method_on_call_result"),
        "1 ABC 3\n2 bc\n"
    );
}

#[test]
fn gc_contract_array_methods_via_call_result() {
    let source = r##"function mk(): any { return [3, 1, 2]; }
async function main(): Promise<void> {
  console.log(JSON.stringify(mk().sort()), mk().indexOf(2), mk().includes(3));
  console.log(mk().join("-"), mk().length, JSON.stringify(mk().reverse()));
  console.log(JSON.stringify(mk().concat([9])), mk().at(-1));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_array_methods_via_call_result"),
        "[1,2,3] 2 true\n3-1-2 3 [2,1,3]\n[3,1,2,9] 2\n"
    );
}

#[test]
fn gc_contract_string_methods_via_call_result() {
    let source = r##"function mk(): any { return { s: "Hello" }; }
async function main(): Promise<void> {
  console.log(mk().s.toLowerCase(), mk().s.length, mk().s.indexOf("l"), mk().s.split("l").length);
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_string_methods_via_call_result"),
        "hello 5 2 3\n"
    );
}

#[test]
fn gc_contract_any_param() {
    let source = r##"function addTo(x: any, v: any): void { x.list.push(v); x.list.unshift(0); }
async function main(): Promise<void> {
  const h: any = { list: [1] };
  addTo(h, "z");
  addTo({ list: [9] }, 1);
  console.log(JSON.stringify(h));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_any_param"),
        "{\"list\":[0,1,\"z\"]}\n"
    );
}

#[test]
fn gc_contract_assign_through_call() {
    let source = r##"let g: any = { list: [1] };
function mk(): any { return g; }
async function main(): Promise<void> {
  mk().list = [5, 6];
  mk().list.push(7);
  mk().count = mk().list.length;
  console.log(JSON.stringify(g));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_assign_through_call"),
        "{\"list\":[5,6,7],\"count\":3}\n"
    );
}

#[test]
fn gc_contract_conditional_receiver() {
    let source = r##"async function main(): Promise<void> {
  const a: any = { list: [1] };
  const b: any = { list: [2] };
  (Math.random() < 2 ? a : b).list.push(3);
  (a || b).list.push(4);
  (a ?? b).list.push(5);
  console.log(JSON.stringify(a), JSON.stringify(b));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_conditional_receiver"),
        "{\"list\":[1,3,4,5]} {\"list\":[2]}\n"
    );
}

#[test]
fn gc_contract_return_any_array() {
    let source = r##"function mk(): any { return [1, 2, 3]; }
function pick(o: any): any { return o.list; }
async function main(): Promise<void> {
  const a = mk();
  a.push("x");
  pick({ list: [1] }).push(2);
  const l = pick({ list: [1] });
  l.push(9);
  console.log(JSON.stringify(a), JSON.stringify(l), a.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_return_any_array"),
        "[1,2,3,\"x\"] [1,9] 4\n"
    );
}

#[test]
fn gc_contract_index_assign_chain() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { rows: [[1], [2]] };
  o.rows[0].push(3);
  o.rows[1][0] = "z";
  o["rows"][0][1] = 7;
  console.log(JSON.stringify(o));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_index_assign_chain"),
        "{\"rows\":[[1,7],[\"z\"]]}\n"
    );
}

#[test]
fn gc_contract_for_in_keys() {
    let source = r##"function mk(): any { return { a: [1], b: [2] }; }
async function main(): Promise<void> {
  const o = mk();
  for (const k in o) o[k].push(0);
  for (const k of Object.keys(mk())) console.log(k);
  console.log(JSON.stringify(o));
  Object.values(o).forEach((v: any) => v.push("v"));
  console.log(JSON.stringify(o));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_for_in_keys"),
        "a\nb\n{\"a\":[1,0],\"b\":[2,0]}\n{\"a\":[1,0,\"v\"],\"b\":[2,0,\"v\"]}\n"
    );
}

#[test]
fn gc_contract_concat_spreads_array_arguments() {
    let source = r##"async function main(): Promise<void> {
  const t: any[] = [3, 1, 2];
  console.log(JSON.stringify(t.concat([9])), JSON.stringify(t.concat(9, [7, 8])), JSON.stringify(t.concat([[5]])));
  const j: any = [1];
  const k: any = [2, 3];
  console.log(JSON.stringify(t.concat(k)), JSON.stringify(j.concat(k)), JSON.stringify(j.concat(k, 5)));
  const n: number[] = [1, 2];
  console.log(JSON.stringify(n.concat([3])), JSON.stringify(n.concat(4)));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_concat_spreads_array_arguments"),
        "[3,1,2,9] [3,1,2,9,7,8] [3,1,2,[5]]\n[3,1,2,2,3] [1,2,3] [1,2,3,5]\n[1,2,3] [1,2,4]\n"
    );
}

#[test]
fn gc_contract_global_any_literal_dynamic_elements() {
    let source = r##"let g: any = { a: [1] };
const h: any = { a: [1] };
const arr: any = [[1]];
async function main(): Promise<void> {
  g.a.push("s");
  h.a.push("t");
  arr[0].push("u");
  console.log(JSON.stringify(g), JSON.stringify(h), JSON.stringify(arr));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_global_any_literal_dynamic_elements"),
        "{\"a\":[1,\"s\"]} {\"a\":[1,\"t\"]} [[1,\"u\"]]\n"
    );
}

#[test]
fn gc_contract_string_methods_on_any() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { s: "Ab", n: 3.14159 };
  console.log(o.s.toLowerCase(), o.s.toUpperCase(), o.s.length, o.s.indexOf("b"), o.s.split("b").length);
  console.log(o.s.slice(1), o.s.trim(), o.s.at(0), o.n.toFixed(2), o.s.charAt(1), o.s.padStart(4, "-"));
  const p: any = "xyz";
  console.log(p.toUpperCase(), p.includes("y"), p.replace("y", "Y"), p.startsWith("x"));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_string_methods_on_any"),
        "ab AB 2 1 2\nb Ab A 3.14 b --Ab\nXYZ true xYz true\n"
    );
}

#[test]
fn gc_contract_mutating_callback_param_on_any_array() {
    let source = r##"async function main(): Promise<void> {
  const arr: any[] = [{ n: 1 }, { n: 2 }];
  arr.forEach((x: any) => { x.n = 5; });
  console.log(JSON.stringify(arr));
  arr.map((x: any) => { x.n++; return x; });
  console.log(JSON.stringify(arr));
  for (const x of arr) x.n = 9;
  console.log(JSON.stringify(arr));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_mutating_callback_param_on_any_array"),
        "[{\"n\":5},{\"n\":5}]\n[{\"n\":6},{\"n\":6}]\n[{\"n\":9},{\"n\":9}]\n"
    );
}

#[test]
fn gc_contract_update_and_compound_on_any_member() {
    let source = r##"function f(x: any): void { x.n = 5; x.list.push(7); }
async function main(): Promise<void> {
  const rows: any = [{ n: 1, list: [1] }];
  f(rows[0]);
  console.log(JSON.stringify(rows));
  const o: any = { n: 1 };
  o.n++;
  o.n += 2;
  o["n"] *= 2;
  console.log(JSON.stringify(o));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_update_and_compound_on_any_member"),
        "[{\"n\":5,\"list\":[1,7]}]\n{\"n\":8}\n"
    );
}

#[test]
fn gc_contract_increment_decrement_on_any_member() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { n: 1, s: "a" };
  o.n++;
  ++o.n;
  o.n--;
  o.s += "b";
  console.log(JSON.stringify(o));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_increment_decrement_on_any_member"),
        "{\"n\":2,\"s\":\"ab\"}\n"
    );
}

#[test]
fn gc_contract_push_spread_of_arrays() {
    let source = r##"async function main(): Promise<void> {
  const nums: number[] = [1, 2];
  const all: number[] = [];
  all.push(...nums);
  const anys: any[] = [];
  anys.push(...nums);
  const j: any = [7, 8];
  anys.push(...j);
  console.log(JSON.stringify(all), JSON.stringify(anys));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_push_spread_of_arrays"),
        "[1,2] [1,2,7,8]\n"
    );
}

#[test]
fn gc_contract_nested_object_with_methods_in_any() {
    let source = r##"async function main(): Promise<void> {
  const m: any = { a: { v: 1, inc() { this.v++; return this.v; } } };
  console.log(m.a.inc(), m.a.v);
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_nested_object_with_methods_in_any"),
        "2 2\n"
    );
}

#[test]
fn gc_contract_foreach_callback_mutates_element() {
    let source = r##"async function main(): Promise<void> {
  const rows: any = [{ list: [1] }, { list: [2] }];
  rows.forEach((x: any) => x.list.push("c"));
  console.log(JSON.stringify(rows));
}
"##;
    assert_eq!(
        compile_and_run(source, "gc_contract_foreach_callback_mutates_element"),
        "[{\"list\":[1,\"c\"]},{\"list\":[2,\"c\"]}]\n"
    );
}
