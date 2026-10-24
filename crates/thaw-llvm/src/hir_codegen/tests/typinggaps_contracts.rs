// Typing gaps: string indexing, Number(union), catch-any property writes, any alias of an async call. Expected output recorded from node.

#[test]
fn typinggaps_string_index_undefined_contract() {
    let source = r##"function main(): void {
  const s = "abc";
  console.log(s[1]);
  console.log(s[10]);
  console.log(s[10] === undefined, s[0] !== undefined, s[-1], s[1.5]);
  const i = 2;
  console.log(s[i] ?? "dflt", s[i + 5] ?? "dflt");
  let n = 0;
  for (let k = 0; k < s.length; k++) { if (s[k] === "b") n++; }
  console.log(n, s[0] + s[2]);
}
"##;
    assert_eq!(compile_and_run(source, "typinggaps_string_index_undefined_contract"), "b\nundefined\ntrue true undefined undefined\nc dflt\n1 ac\n");
}

#[test]
fn typinggaps_number_of_numeric_union_comparator_contract() {
    let source = r##"function main(): void {
  const xs: (number | string)[] = [3, "1", 2];
  xs.sort((a, b) => Number(a) - Number(b));
  console.log(xs);
  const ys: (number | undefined)[] = [3, 1, 2];
  ys.sort((a, b) => a! - b!);
  console.log(ys, Number(xs[0]) + 1);
}
"##;
    assert_eq!(compile_and_run(source, "typinggaps_number_of_numeric_union_comparator_contract"), "[ '1', 2, 3 ]\n[ 1, 2, 3 ] 2\n");
}

#[test]
fn typinggaps_catch_any_property_assign_contract() {
    let source = r##"function main(): void {
  try { throw new TypeError("boom"); } catch (e: any) { e.k = 2; e.code = "E1"; console.log(e.k, e.code, e.message, e.name); }
  try { throw { a: 1 }; } catch (e: any) { e.k = 2; console.log(e.k, e.a); }
  try { throw new Error("rethrow"); } catch (e: any) { e.k = 1; try { throw e; } catch (e2: any) { console.log(e2.k, e2.message); } }
}
"##;
    assert_eq!(compile_and_run(source, "typinggaps_catch_any_property_assign_contract"), "2 E1 boom TypeError\n2 1\n1 rethrow\n");
}

#[test]
fn typinggaps_any_alias_of_async_call_contract() {
    let source = r##"async function g(): Promise<number> { return 5; }
async function main(): Promise<void> {
  const p: any = g();
  console.log(await p);
  const q: any = g();
  q.then((v: number) => console.log("then", v));
  const r = await q;
  console.log(r + 1);
}
"##;
    assert_eq!(compile_and_run(source, "typinggaps_any_alias_of_async_call_contract"), "5\nthen 5\n6\n");
}
