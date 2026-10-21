// Wave LA: structuredClone of host Map/Set rebuilds collections, functions are rejected with
// DataCloneError, and sparse host arrays are cloned without probing every index. Expected output
// from node 22.

#[test]
fn la_contract_structured_clone_rebuilds_host_map_and_set() {
    let source = r##"async function main(): Promise<void> {
  const m: any = new Map([[1, 2]]);
  const mc: any = structuredClone(m);
  console.log(mc === m, mc instanceof Map, mc.get(1), mc.size);
  m.set(3, 4);
  mc.set(9, 9);
  console.log(mc.size, m.size, mc.has(9), m.has(9));
  const s: any = new Set([1, 2]);
  const sc: any = structuredClone(s);
  sc.add(7);
  console.log(sc instanceof Set, sc.size, s.size, sc.has(7), s.has(7));
  const nested: any = new Map([[{ k: 1 }, [1, 2]]]);
  const nc: any = structuredClone(nested);
  let kk: any = null;
  let vv: any = null;
  nc.forEach((v: any, k: any) => { kk = k; vv = v; });
  console.log(kk.k, vv.length, vv !== nested.get([...nested.keys()][0]));
}
"##;
    assert_eq!(
        compile_and_run(source, "la_contract_structured_clone_rebuilds_host_map_and_set"),
        "false true 2 1\n2 2 true false\ntrue 3 2 true false\n1 2 true\n"
    );
}

#[test]
fn la_contract_structured_clone_rejects_functions_with_data_clone_error() {
    let source = r##"async function main(): Promise<void> {
  const f: any = (x: number) => x;
  let name = "none";
  let ok = false;
  try { structuredClone(f); } catch (e: any) { name = e.name; ok = e.message.endsWith("could not be cloned."); }
  console.log(name, ok);
  let inner = "none";
  try { structuredClone({ f }); } catch (e: any) { inner = e.name; }
  console.log(inner);
}
"##;
    assert_eq!(
        compile_and_run(source, "la_contract_structured_clone_rejects_functions_with_data_clone_error"),
        "DataCloneError true\nDataCloneError\n"
    );
}

#[test]
fn la_contract_structured_clone_of_sparse_host_array_keeps_holes() {
    let source = r##"async function main(): Promise<void> {
  const a: any = new Array(50000);
  a[5] = "x";
  a[49999] = { k: 1 };
  const c: any = structuredClone({ a });
  console.log(c.a.length, 0 in c.a, 5 in c.a, c.a[5], c.a[49999].k, c.a[49999] !== a[49999]);
  const b: any = [1, , 3, undefined];
  const bc: any = structuredClone(b);
  console.log(bc.length, 1 in bc, 3 in bc, bc[3], bc[2]);
}
"##;
    assert_eq!(
        compile_and_run(source, "la_contract_structured_clone_of_sparse_host_array_keeps_holes"),
        "50000 false true x 1 true\n4 false true undefined 3\n"
    );
}
