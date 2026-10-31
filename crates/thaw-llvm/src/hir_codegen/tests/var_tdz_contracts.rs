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

#[test]
fn var_tdz_closure_flag_function_level() {
    let source = r##"function main(): void {
  const g = () => k;
  let k = 3;
  console.log(g());
  function early(): number { return z + 1; }
  try { early(); } catch (e) { console.log((e as Error).message); }
  let z = 1;
  console.log(early());
}
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_closure_flag_function_level"),
        "3\nCannot access 'z' before initialization\n2\n"
    );
}

#[test]
fn var_tdz_typeof_before_let() {
    let source = r##"function main(): void {
  try { console.log(typeof w); } catch (e) { console.log((e as Error).name); }
  let w = 1;
  console.log(typeof w);
}
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_typeof_before_let"),
        "ReferenceError\nnumber\n"
    );
}

#[test]
fn var_tdz_var_in_for_head_read_after() {
    let source = r##"function main(): void {
  for (var i = 0; i < 3; i++) {}
  console.log(i);
}
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_var_in_for_head_read_after"),
        "3\n"
    );
}

#[test]
fn var_tdz_module_level() {
    let source = r##"console.log(typeof h);
var h = 3;
console.log(h);
try { console.log(x); } catch (e) { console.log((e as Error).name, (e as Error).message); }
let x = 5;
console.log(x);
function f(): number {
  try { return y + 1; } catch (e) { console.log((e as Error).message); return -1; }
}
console.log(f());
let y = 2;
console.log(f());
function useLater(): number { return z; }
let z = 7;
console.log(useLater());
const w = () => q;
let q = 1;
console.log(w());
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_module_level"),
        "undefined\n3\nReferenceError Cannot access 'x' before initialization\n5\nCannot access 'y' before initialization\n-1\n3\n7\n1\n"
    );
}

#[test]
fn var_tdz_module_level_new_before_class() {
    let source = r##"try { const a = new D(); console.log(a.v); } catch (e) { console.log((e as Error).name, (e as Error).message); }
class D { v = 1; }
console.log(new D().v);
"##;
    assert_eq!(
        compile_and_run(source, "var_tdz_module_level_new_before_class"),
        "ReferenceError Cannot access 'D' before initialization\n1\n"
    );
}
