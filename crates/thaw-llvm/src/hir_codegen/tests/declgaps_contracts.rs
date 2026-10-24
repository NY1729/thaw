
#[test]
fn declgaps_uninitialized_scalar_declarations_assigned_later() {
    let source = r###"let gs: string;
let gn: number;
function setup(): void { gs = "g"; gn = 2; }
function main(): void {
  setup();
  let p: string;
  let n: number;
  let flag: boolean;
  p = "hi";
  n = 4;
  if (n > 3) { flag = true; } else { flag = false; }
  console.log(p, n + 1, flag, gs, gn);
}
"###;
    assert_eq!(compile_and_run(source, "declgaps_uninit"), "hi 5 true g 2\n");
}

#[test]
fn declgaps_loop_bindings_do_not_clash_with_later_top_level_consts() {
    let source = r###"const o: any = { a: 1, b: 2 };
for (const k in o) { console.log(k); }
const k = 5;
console.log(k);
const xs = [1, 2];
for (const s of xs) { const d = s * 2; console.log(d); }
const d = 9;
console.log(d);
for (const j of [7]) { console.log(j); }
const j = 8;
console.log(j);
"###;
    assert_eq!(compile_and_run(source, "declgaps_loop_scope"), "a\nb\n5\n2\n4\n9\n7\n8\n");
}

#[test]
fn declgaps_any_then_callback_does_not_corrupt_typed_then_callback() {
    let source = r###"async function g(): Promise<number> { return 7; }
const p = g();
p.then((v: number) => console.log("a", v));
p.then((v: any) => console.log("b", v));
function main(): void {
  g().then((v: number) => console.log("c", v + 1));
  g().then((v: any) => console.log("d", v));
}
"###;
    assert_eq!(compile_and_run(source, "declgaps_then_any"), "a 7\nb 7\nc 8\nd 7\n");
}

#[test]
fn declgaps_recursive_local_const_arrows() {
    let source = r###"function main(): void {
  const even = (n: number): boolean => n === 0 ? true : odd(n - 1);
  const odd = (n: number): boolean => n === 0 ? false : even(n - 1);
  console.log(even(10), odd(7), even(3));
  let calls = 0;
  const fact = (n: number): number => { calls++; return n <= 1 ? 1 : n * fact(n - 1); };
  console.log(fact(5), calls);
}
"###;
    assert_eq!(compile_and_run(source, "declgaps_local_arrows"), "true true false\n120 5\n");
}

#[test]
fn declgaps_number_bigint_union_number_conversion() {
    let source = r###"function add(x: number | bigint): number { return Number(x) + 1; }
function main(): void {
  const a: number | bigint = 5n;
  const b: number | bigint = 3;
  console.log(Number(a) + Number(b), add(5n), add(2));
}
"###;
    assert_eq!(compile_and_run(source, "declgaps_bigint_union"), "8 6 3\n");
}
