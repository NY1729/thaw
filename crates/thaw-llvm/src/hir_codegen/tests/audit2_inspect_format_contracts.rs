// Node-verified contracts for audit2 group D: U+0001 strings, util.format / console format,
// util.inspect of BigInt/typed arrays/Buffer, console.table(Map/Set), symbol-key display.
// Expected text recorded from Node v22.

#[test]
fn audit2_inspect_format_u0001_prefixed_strings_stay_plain() {
    let source = r####"function main(): void {
  const s: string = "\u0001abc";
  console.log(s.length, s === "\u0001abc", JSON.stringify(s), s.charCodeAt(0));
  console.log(["\u0001", "\u0001x"], { k: "\u0001y" });
  console.log("\u0001q");
}
"####;
    assert_eq!(compile_and_run(source, "audit2_inspect_format_u0001_prefixed_strings_stay_plain"), r####"4 true "\u0001abc" 1
[ '\x01', '\x01x' ] { k: '\x01y' }
q
"####);
}

#[test]
fn audit2_inspect_format_console_format_nullish_bigint_symbol() {
    let source = r####"function main(): void {
  console.log("%s", null, undefined, 5n, Symbol("q"));
  console.log("%d", 5n, "%i", 7n);
  console.log("%s", 5n);
  console.log("%s", Symbol("r"));
}
"####;
    assert_eq!(compile_and_run(source, "audit2_inspect_format_console_format_nullish_bigint_symbol"), r####"null undefined 5n Symbol(q)
5n %i 7n
5n
Symbol(r)
"####);
}

#[test]
fn audit2_inspect_format_console_table_map_set() {
    let source = r####"function main(): void {
  console.table(new Map<string, number>([["k", 1], ["j", 2]]));
  console.table(new Set<number>([1, 2]));
}
"####;
    assert_eq!(compile_and_run(source, "audit2_inspect_format_console_table_map_set"), r####"┌───────────────────┬─────┬────────┐
│ (iteration index) │ Key │ Values │
├───────────────────┼─────┼────────┤
│ 0                 │ 'k' │ 1      │
│ 1                 │ 'j' │ 2      │
└───────────────────┴─────┴────────┘
┌───────────────────┬────────┐
│ (iteration index) │ Values │
├───────────────────┼────────┤
│ 0                 │ 1      │
│ 1                 │ 2      │
└───────────────────┴────────┘
"####);
}

#[test]
fn audit2_inspect_format_console_log_symbol_keys() {
    let source = r####"function main(): void {
  const k = Symbol("k");
  const o: any = { a: 2 };
  o[k] = 1;
  console.log(o);
  const p = { a: 2, [k]: 1 };
  console.log(p);
}
"####;
    assert_eq!(compile_and_run(source, "audit2_inspect_format_console_log_symbol_keys"), r####"{ a: 2, [Symbol(k)]: 1 }
{ a: 2, [Symbol(k)]: 1 }
"####);
}
