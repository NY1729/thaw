// Wave GA: an assertion to `any` / `any[]` / `unknown[]` makes an array literal's elements dynamic.
// Expected output from node 22 (type assertions are erased).

#[test]
fn ga_contract_as_any_array_literal_takes_any_element_type() {
    let source = r##"function f(arr: any): number { arr.push("p"); return arr.length; }
async function main(): Promise<void> {
  const h: any = { items: [1, 2] as any[] };
  h.items.push("x");
  h.items.push({ a: 1 });
  h.items.push(null);
  h.items[0] = "z";
  console.log(h.items.length, JSON.stringify(h.items), typeof h.items[2], h.items[2].a);
  const a = [1, 2] as any;
  a.push("s");
  a.push([9]);
  a[0] = { q: 1 };
  console.log(a.length, JSON.stringify(a), a[0].q);
  const b = <any[]>[1, 2];
  b.push("t");
  b.unshift(null);
  console.log(JSON.stringify(b));
  console.log(f([1, 2] as any[]), f(["a"] as any));
}
"##;
    assert_eq!(
        compile_and_run(source, "ga_contract_as_any_array_literal_takes_any_element_type"),
        "5 [\"z\",2,\"x\",{\"a\":1},null] string undefined\n4 [{\"q\":1},2,\"s\",[9]] 1\n[null,1,2,\"t\"]\n3 2\n"
    );
}

#[test]
fn ga_contract_nested_and_unknown_assertions() {
    let source = r##"async function main(): Promise<void> {
  const n: any = { a: { b: [1] as any[] }, l: [[1] as any[], 2] as any[] };
  n.a.b.push("deep");
  n.l[0].push("in");
  n.l.push("out");
  console.log(JSON.stringify(n));
  const u = [1, 2] as unknown[];
  u.push("w");
  u.splice(1, 1, "sp", { z: 1 });
  u.fill("f", 0, 1);
  console.log(JSON.stringify(u), u.length);
  const g = ([3, 1, 2] as any[]);
  g.push("x");
  console.log(Array.isArray(g), g instanceof Array, g.map((v: any) => typeof v).join(","));
  const r: any = { items: [1, 2] as any[] };
  const view = r;
  view.items.push("alias");
  console.log(JSON.stringify(r), JSON.stringify(view));
}
"##;
    assert_eq!(
        compile_and_run(source, "ga_contract_nested_and_unknown_assertions"),
        "{\"a\":{\"b\":[1,\"deep\"]},\"l\":[[1,\"in\"],2,\"out\"]}\n[\"f\",\"sp\",{\"z\":1},\"w\"] 4\ntrue true number,number,number,string\n{\"items\":[1,2,\"alias\"]} {\"items\":[1,2,\"alias\"]}\n"
    );
}

#[test]
fn ga_contract_assertion_sinks_and_typed_regressions() {
    let source = r##"function g(): any { return [7] as any[]; }
async function main(): Promise<void> {
  (({ a: [1] as any[] }) as any).a.push("x");
  const o = { list: [1] } as any;
  o.list.push("y");
  (o as any).list.push("w");
  console.log(JSON.stringify(o));
  const h: any = {};
  h.x = [1, 2] as any[];
  h.x.push("z");
  console.log(JSON.stringify(h));
  const r = g();
  r.push("r");
  console.log(JSON.stringify(r));
  const mix = [1, "a", null] as any[];
  mix.push({ k: 1 });
  console.log(JSON.stringify(mix), mix.length);
  const m = [[1, 2], [3]] as any[];
  m[0].push("q");
  m.push("w");
  console.log(JSON.stringify(m));
  const u = [1, 2] as unknown as any[];
  u.push("u");
  console.log(JSON.stringify(u));
  const t = [1, 2] as number[];
  t.push(3);
  console.log(t.length);
  const arr = [1, 2];
  const v = arr as any;
  v.push(3);
  console.log(arr.length, JSON.stringify(v));
  const inner: any = { deep: { l: [1] as any[] } };
  inner.deep.l.push("d");
  console.log(JSON.stringify(inner));
}
"##;
    assert_eq!(
        compile_and_run(source, "ga_contract_assertion_sinks_and_typed_regressions"),
        "{\"list\":[1,\"y\",\"w\"]}\n{\"x\":[1,2,\"z\"]}\n[7,\"r\"]\n[1,\"a\",null,{\"k\":1}] 4\n[[1,2,\"q\"],[3],\"w\"]\n[1,2,\"u\"]\n3\n3 [1,2,3]\n{\"deep\":{\"l\":[1,\"d\"]}}\n"
    );
}
