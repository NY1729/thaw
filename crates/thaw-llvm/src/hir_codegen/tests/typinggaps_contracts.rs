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
fn typinggaps_saved_string_index_preserves_undefined() {
    let source = r##"function main(): void {
  const s = "abc"[99];
  console.log(s === undefined, typeof s, String(s));
}
"##;
    assert_eq!(
        compile_and_run(source, "typinggaps_saved_string_index_preserves_undefined"),
        "true undefined undefined\n"
    );
}

#[test]
fn typinggaps_string_index_value_flow_contract() {
    let source = r##"function missing(): string | undefined { return "abc"[99]; }
function base(): string { console.log("base"); return "abc"; }
function index(): number { console.log("index"); return 99; }
function main(): void {
  const text = "abc";
  const absentConst = text[99];
  let absentLet = text[text.length];
  const returned = missing();
  console.log("const", absentConst === undefined, typeof absentConst, String(absentConst));
  console.log("let", absentLet === undefined, typeof absentLet, absentLet ?? "fallback", absentLet?.length);
  console.log("convert", String(text[99]), "prefix:" + text[99], "prefix:" + absentConst);
  console.log("return", returned === undefined, typeof returned, String(returned));
  absentLet = text[1];
  console.log("assigned", absentLet, typeof absentLet);
  const dynamicKey: string = "1";
  console.log("keys", text[0], text["1"], text[dynamicKey], text[3] === undefined,
    text["99"] === undefined, text["01"] === undefined, text[-1] === undefined,
    text[1.5] === undefined, ""[0] === undefined);
  console.log("charAt", text.charAt(99) === "", text.charAt(99));
  const once = base()[index()];
  console.log("once", once === undefined);
}
"##;
    assert_eq!(
        compile_and_run(source, "typinggaps_string_index_value_flow_contract"),
        "const true undefined undefined\nlet true undefined fallback undefined\nconvert undefined prefix:undefined prefix:undefined\nreturn true undefined undefined\nassigned b string\nkeys a b b true true true true true true\ncharAt true \nbase\nindex\nonce true\n"
    );
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
