// Wave FE: Json boxes handed to native callbacks outlive the call when stored (index write, push),
// and a Dictionary crossing into `any` is an alias. Expected output from node 22.

#[test]
fn fe_contract_any_array_view_writes_keep_stored_json_alive() {
    let source = r##"async function main(): Promise<void> {
  const arr: any[] = [1, 2, 3];
  const v: any = arr;
  v[0] = { a: 1 };
  v[5] = [9, 8];
  console.log(JSON.stringify(arr), arr.length);
  v.push({ p: 1 }, "s");
  v.unshift({ u: 1 });
  console.log(JSON.stringify(arr), arr.length);
  v.splice(1, 2, { s: 1 }, [3]);
  console.log(JSON.stringify(arr), arr.length);
  v.fill({ f: 1 }, 0, 2);
  v.length = 3;
  v.reverse();
  console.log(JSON.stringify(arr), arr.length);
  const nested: any[][] = [[1], [2]];
  const nv: any = nested;
  nv[0][0] = { n: 1 };
  nv[1].push({ m: 2 });
  console.log(JSON.stringify(nested));
  const items: any[] = [1, 2];
  const holder = { items };
  const hv: any = holder;
  hv.items[0] = { h: 1 };
  hv.items.push({ h: 2 });
  console.log(JSON.stringify(holder.items));
  const late: any[] = [];
  const lv: any = late;
  for (let i = 0; i < 3; i++) {
    await new Promise<void>((resolve) => setTimeout(() => resolve(), 5));
    lv.push({ turn: i, pad: "x".repeat(100) });
  }
  console.log(JSON.stringify(late.map((e: any) => e.turn)), arr.length);
}
"##;
    assert_eq!(
        compile_and_run(source, "fe_contract_any_array_view_writes_keep_stored_json_alive"),
        "[{\"a\":1},2,3,null,null,[9,8]] 6\n[{\"u\":1},{\"a\":1},2,3,null,null,[9,8],{\"p\":1},\"s\"] 9\n[{\"u\":1},{\"s\":1},[3],3,null,null,[9,8],{\"p\":1},\"s\"] 9\n[[3],{\"f\":1},{\"f\":1}] 3\n[[{\"n\":1}],[2,{\"m\":2}]]\n[{\"h\":1},2,{\"h\":2}]\n[0,1,2] 3\n"
    );
}

#[test]
fn fe_contract_native_callback_storing_json_parameter_in_native_array() {
    let source = r##"async function main(): Promise<void> {
  const store: any[] = [];
  const f = (j: any, k: any): void => { store[0] = j; store.push(k); };
  const g: any = f;
  g({ a: 1 }, [2]);
  await new Promise<void>((resolve) => setTimeout(() => resolve(), 5));
  g({ b: 3 }, { c: 4 });
  await new Promise<void>((resolve) => setTimeout(() => resolve(), 5));
  console.log(JSON.stringify(store), store.length);
  const seeded: any[] = [1];
  const h = (j: any): void => { seeded[0] = j; };
  const hg: any = h;
  hg({ z: 9 });
  console.log(JSON.stringify(seeded));
}
"##;
    assert_eq!(
        compile_and_run(source, "fe_contract_native_callback_storing_json_parameter_in_native_array"),
        "[{\"b\":3},[2],{\"c\":4}] 3\n[{\"z\":9}]\n"
    );
}

#[test]
fn fe_contract_dictionary_aliased_to_any_stays_valid() {
    let source = r##"async function main(): Promise<void> {
  const d: Record<string, number> = { a: 1 };
  const dv: any = d;
  console.log(d.a, dv.a);
  dv.a = 7;
  dv.c = 5;
  console.log(d.a, JSON.stringify(d), JSON.stringify(dv));
  const e: Record<string, any> = { a: 1 };
  const ev: any = e;
  ev.a = { x: 1 };
  ev.b = [2];
  console.log(JSON.stringify(e), e.a === ev.a);
}
"##;
    assert_eq!(
        compile_and_run(source, "fe_contract_dictionary_aliased_to_any_stays_valid"),
        "1 1\n7 {\"a\":7,\"c\":5} {\"a\":7,\"c\":5}\n{\"a\":{\"x\":1},\"b\":[2]} true\n"
    );
}
