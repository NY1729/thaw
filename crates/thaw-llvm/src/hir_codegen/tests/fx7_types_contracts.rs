// Round-4 type contracts (fx7): sort/toSorted receiver typing, arithmetic on dynamic/Date operands,
// BigInt-number mixing, await-member array calls, and instanceof on builtin global constructors.
// Expected output is from node 22 (`node --experimental-transform-types`).

#[test]
fn fx7_sort_result_feeds_reverse_match_node() {
    let source = r#"
function main(): void {
  console.log([3, 1, 2].sort().reverse().join(","));
  console.log(["b", "a"].sort().reverse().join(","));
  console.log([3, 1, 2].toSorted().reverse().join(","));
  console.log([3, 1, 2].sort().reverse().length);
}
"#;
    assert_eq!(
        compile_and_run(source, "fx7_sort_result_feeds_reverse"),
        "3,2,1\nb,a\n3,2,1\n3\n"
    );
}

#[test]
fn fx7_dynamic_and_date_arithmetic_match_node() {
    let source = r#"
function sub(a: any, b: any) { return a - b; }
function main(): void {
  console.log(sub(5, 2), sub("5", 2));
  const a = new Date(1000);
  const b = new Date(0);
  console.log((a as any) - (b as any));
  console.log((new Date(1000) as any) * 2, (new Date(1000) as any) - 1);
  const t = new Date(0);
  console.log(((new Date()) as any) - (t as any) > 0);
}
"#;
    assert_eq!(
        compile_and_run(source, "fx7_dynamic_and_date_arithmetic"),
        "3 3\n1000\n2000 999\ntrue\n"
    );
}

#[test]
fn fx7_bigint_mixed_with_number_throws_type_error() {
    let source = r#"
function main(): void {
  try { console.log((1n as any) + 1); } catch (e: any) { console.log(e.name + ": " + e.message); }
  try { console.log(1n + (1 as any)); } catch (e: any) { console.log(e.name + ": " + e.message); }
  try { console.log((1n as any) * 2); } catch (e: any) { console.log(e.name + ": " + e.message); }
}
"#;
    assert_eq!(
        compile_and_run(source, "fx7_bigint_mixed_throws"),
        "TypeError: Cannot mix BigInt and other types, use explicit conversions\n\
TypeError: Cannot mix BigInt and other types, use explicit conversions\n\
TypeError: Cannot mix BigInt and other types, use explicit conversions\n"
    );
}

#[test]
fn fx7_instanceof_builtin_object_function_and_primitive_wrappers_match_node() {
    let source = r#"
function main(): void {
  const v = new Map();
  console.log(v instanceof Object, v instanceof Function, v instanceof Promise);
  const f = () => 1;
  console.log(f instanceof Function, f instanceof Object);
  const s: any = "x";
  console.log(s instanceof String, 5 instanceof Number);
}
"#;
    assert_eq!(
        compile_and_run(source, "fx7_instanceof_builtins"),
        "true false false\ntrue true\nfalse false\n"
    );
}

#[test]
fn fx7_await_member_and_any_array_calls_match_node() {
    let source = r#"
async function arr(): Promise<any> {
  return JSON.parse("[3,1,2]");
}
async function listed(): Promise<void> {
  console.log((await arr()).sort().join(","));
}
function main(): void {
  const x = Array.from([1, 2], (v: any) => v);
  const y = new Array(3).fill(0);
  console.log(x.join(","), y.join(","));
  listed();
}
"#;
    assert_eq!(
        compile_and_run(source, "fx7_await_member_and_any_array_calls"),
        "1,2 0,0,0\n1,2,3\n"
    );
}
