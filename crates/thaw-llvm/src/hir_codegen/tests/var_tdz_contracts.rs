// `var` hoisting and straight-line TDZ. Expected output recorded from node 22.

#[test]
fn var_tdz_hoisted_read_in_function() {
    let source = r##"function f(): void {
  console.log(a);
  var a = 1;
  console.log(a);
}
f();
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_hoisted_read_in_function"),
        "undefined\n1\n"
    );
}

#[test]
fn var_tdz_var_in_branch_read_outside() {
    let source = r##"function g(n: number): number {
  if (n > 0) { var r = n * 2; }
  return r === undefined ? -1 : r;
}
console.log(g(1), g(0));
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_var_in_branch_read_outside"),
        "2 -1\n"
    );
}

#[test]
fn var_tdz_let_const_straight_line() {
    let source = r##"function main(): void {
  try { console.log(x); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  let x = 5;
  console.log(x);
  try { console.log(c + 1); } catch (e) { console.log((e as Error).message); }
  const c = 2;
  console.log(c);
}
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_let_const_straight_line"),
        "ReferenceError Cannot access 'x' before initialization\n5\nCannot access 'c' before initialization\n2\n"
    );
}

#[test]
fn var_tdz_hoist_closure_loop_typeof() {
    let source = r##"function f(): void {
  const g = () => k;
  var k = 4;
  console.log(g());
}
function h(n: number): number {
  for (let i = 0; i < 2; i++) { if (i === 1) { var last = i + n; } }
  return last === undefined ? -1 : last;
}
function w(flag: boolean): string {
  console.log(typeof q);
  if (flag) { var q = "set"; }
  return String(q);
}
function main(): void {
  f();
  console.log(h(5), w(true), w(false));
}
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_hoist_closure_loop_typeof"),
        "4\nundefined\nundefined\n6 set undefined\n"
    );
}

#[test]
fn var_tdz_multi_declarator_redeclare() {
    let source = r##"function main(): void {
  console.log(a, b);
  var a = 1, b = 2;
  var a = 3;
  console.log(a, b);
  var plain = 7;
  console.log(plain);
}
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_multi_declarator_redeclare"),
        "undefined undefined\n3 2\n7\n"
    );
}
