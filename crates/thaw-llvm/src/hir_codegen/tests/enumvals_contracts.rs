// Contracts for enums and namespaces used as values: `const E2 = E`, Object.keys/values/entries, for...in,
// `typeof E` parameters, `keyof typeof E` indexing, `E[stringKey]`, and the same for namespaces.
// Expected output from node.

#[test]
fn enumvals_enum_object_as_value() {
    let source = r###"enum E { A, B, C }
enum S { X = "x", Y = "y" }
type K = keyof typeof E;
function f(e: typeof E): number { return e.C; }
function main(): void {
  const E2 = E;
  console.log(Object.keys(E).join(","));
  console.log(Object.values(E).join(","));
  console.log(E[E.A], E[2], E2.B, f(E));
  for (const name in E) console.log(name);
  const k: K = "B";
  console.log(k, E[k]);
  console.log(Object.keys(S).join(","), Object.values(S).join(","));
  for (const q in S) console.log(q);
  console.log(Object.entries(E).length);
}
"###;
    assert_eq!(compile_and_run(source, "enumvals_enum_object_as_value"), "0,1,2,A,B,C\nA,B,C,0,1,2\nA C 1 2\n0\n1\n2\nA\nB\nC\nB 1\nX,Y x,y\nX\nY\n6\n");
}

#[test]
fn enumvals_namespace_object_as_value() {
    let source = r###"namespace N {
  export function fn(): number { return 7; }
  export const v = 3;
  export namespace Inner { export const w = 9; }
}
function g(n: typeof N): number { return n.fn() + n.v + n.Inner.w; }
function main(): void {
  const N2 = N;
  console.log(N2.fn(), N2.v, N2.Inner.w);
  console.log(g(N));
  console.log(Object.keys(N).join(","));
}
"###;
    assert_eq!(compile_and_run(source, "enumvals_namespace_object_as_value"), "7 3 9\n19\nfn,v,Inner\n");
}
