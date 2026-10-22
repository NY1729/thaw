#[test]
fn symkeys_contract_string_enumeration_skips_symbol_keys() {
    let source = r##"function main() {
  const key = Symbol("secret");
  const o = { visible: 1, [key]: 2 };
  console.log(Object.keys(o).join(","));
  console.log(Object.values(o).join(","));
  console.log(Object.entries(o).map((e) => e.join(":")).join(","));
  const seen: string[] = [];
  for (const k in o) seen.push(k);
  console.log(seen.join(","));
  console.log(JSON.stringify(o));
  const dyn: any = o;
  console.log(Object.keys(dyn).join(","), dyn[key]);
  const anon = Symbol();
  const p = { a: 1, [anon]: "x" };
  console.log(Object.keys(p).join(","));
  console.log(Object.keys({ ...p }).join(","));
  console.log(Object.keys(Object.assign({}, p)).join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "symkeys_contract_string_enumeration"),
        "visible\n1\nvisible:1\nvisible\n{\"visible\":1}\nvisible 2\na\na\na\n"
    );
}
