// Audit2 group B: return-path analysis, Math.max spreads, checked native bigint.
// Expected output recorded from node 22.

#[test]
fn audit2_contract_forever_loops_and_grouped_switch_return_on_all_paths() {
    let source = r##"function h(n: number): string { while (true) { if (n++ > 3) return "w" + n; } }
function f(n: number): string { for (;;) { if (n > 2) return "x"; n++; } }
function g(n: number): string { do { if (n > 2) return "y"; n++; } while (true); }
function s(n: number): string { switch (n) { case 1: case 2: return "12"; default: return "d"; } }
function main(): void { console.log(h(0), f(0), g(0), s(1), s(2), s(3)); }
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_forever_loops_and_grouped_switch"),
        "w5 x y 12 12 d\n"
    );
}

#[test]
fn audit2_contract_math_max_spread_with_extra_arguments_and_sets() {
    let source = r##"function main(): void {
  const a = [1, 2]; const s = new Set([1, 5]);
  console.log(Math.max(...a, 3), Math.min(9, ...a), Math.max(...s), Math.max(...[1, 5], 10));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_math_max_spread"),
        "3 1 5 10\n"
    );
}

#[test]
fn audit2_contract_native_bigint_overflow_throws_instead_of_wrapping() {
    let source = r##"function main(): void {
  console.log(2n ** 62n, 3n * 4n, 1n << 62n);
  try { console.log(2n ** 64n); } catch (e) { console.log((e as Error).name); }
  try { let x = 1n; let i = 1n; while (i < 30n) { x = x * i; i = i + 1n; } console.log(x); } catch (e) { console.log((e as Error).name); }
  try { console.log(1n << 70n); } catch (e) { console.log((e as Error).name); }
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_bigint_overflow"),
        "4611686018427387904n 12n 4611686018427387904n\nRangeError\nRangeError\nRangeError\n"
    );
}

#[test]
fn audit2_contract_dictionary_array_values_are_shared_and_mutable() {
    let source = r##"function main(): void {
  const o: { [k: string]: number[] } = { x: [1] };
  o.x.push(2);
  console.log(o);
  const g: Record<string, string[]> = {};
  g["a"] = ["x"]; g["a"].push("y"); console.log(g);
  const groups: Record<string, string[]> = {};
  for (const w of ["apple", "avocado", "banana"]) { const k = w[0]; const l = groups[k] || (groups[k] = []); l.push(w); }
  console.log(groups);
  const m: Record<string, number[]> = {};
  (m["k"] ??= []).push(1); (m["k"] ??= []).push(2);
  console.log(m, m.k.length, m["k"][1] + 1);
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_dictionary_array_values"),
        "{ x: [ 1, 2 ] }\n{ a: [ 'x', 'y' ] }\n{ a: [ 'apple', 'avocado' ], b: [ 'banana' ] }\n{ k: [ 1, 2 ] } 2 3\n"
    );
}

#[test]
fn audit2_contract_any_array_cursor_keeps_nested_links() {
    let source = r##"function main(): void {
  const root: any[] = []; let cur = root;
  for (let i = 0; i < 3; i++) { const n: any[] = []; cur.push(n); cur = n; }
  console.log(JSON.stringify(root));
  const r: any[] = []; const n: any[] = []; r.push(n); n.push(1); console.log(JSON.stringify(r));
}
"##;
    assert_eq!(
        compile_and_run(source, "audit2_contract_any_array_cursor"),
        "[[[[]]]]\n[[1]]\n"
    );
}
