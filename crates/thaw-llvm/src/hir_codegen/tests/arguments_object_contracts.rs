// Surplus call arguments are evaluated and dropped; `arguments` sees them. Expected output from node 22.

#[test]
fn surplus_call_arguments_are_dropped_and_arguments_object_sees_them() {
    let source = r####"function f(a: number, b: number) { return arguments.length; }
function g(a: number) { return arguments[1] + a; }
function h(): number { return Array.from(arguments).length; }
function s(): number { let t = 0; for (let i = 0; i < arguments.length; i++) t += arguments[i]; return t; }
function main(): void {
  console.log((f as any)(1, 2, 3));
  console.log((g as any)(1, 2));
  console.log((h as any)(5, 6, 7, 8));
  console.log((s as any)(1, 2, 3, 4));
  console.log((function () { return Array.prototype.slice.call(arguments, 1); })(1, 2, 3));
  const k = (a: number) => a * 2;
  const log: number[] = [];
  console.log((k as any)(4, log.push(7)), log);
}
"####;
    assert_eq!(compile_and_run(source, "surplus_call_arguments_are_dropped"), r####"3
3
4
10
[ 2, 3 ]
8 [ 7 ]
"####);
}
