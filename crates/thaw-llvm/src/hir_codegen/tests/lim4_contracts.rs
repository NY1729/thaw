// Known Node-22 divergences closed in the fourth "known limits" round. Expected output is real
// node 22 output (`node --experimental-strip-types`).

#[test]
fn lim4_assigned_stack_on_a_class_instance_is_an_enumerable_own_property() {
    let source = r#"class Q { v: number = 1; }
function main(): void {
  const q = new Q();
  (q as any).stack = "x";
  console.log(JSON.stringify(q));
  console.log(q);
  const p = new Q();
  Error.captureStackTrace(p);
  console.log(JSON.stringify(p), Object.keys(p));
}
"#;
    assert_eq!(
        compile_and_run(source, "lim4_assigned_stack_on_a_class_instance"),
        "{\"v\":1,\"stack\":\"x\"}\nQ { v: 1, stack: 'x' }\n{\"v\":1} [ 'v' ]\n"
    );
}

#[test]
fn lim4_delete_stack_reads_undefined() {
    let source = r#"function main(): void {
  const f = new Error("m");
  console.log(typeof f.stack, f.stack !== undefined);
  delete (f as any).stack;
  console.log(f.stack, typeof f.stack);
  const g = new TypeError("t");
  console.log(typeof g.stack, g.stack === undefined);
  try { throw new Error("y"); } catch (e: any) {
    e.stack = "s";
    console.log(e.stack);
    delete e.stack;
    console.log(e.stack, typeof e.stack);
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "lim4_delete_stack_reads_undefined"),
        "string true\nundefined undefined\nstring false\ns\nundefined undefined\n"
    );
}

#[test]
fn lim4_spread_of_a_class_instance_is_a_plain_object() {
    let source = r#"class P { a: number = 1; b: string = "x"; }
function main(): void {
  const p = new P();
  const s = { ...p };
  console.log(s);
  console.log({ z: 0, ...p });
  console.log({ ...p, c: 3 });
  console.log(p, s instanceof P, p instanceof P);
}
"#;
    assert_eq!(
        compile_and_run(source, "lim4_spread_of_a_class_instance"),
        "{ a: 1, b: 'x' }\n{ z: 0, a: 1, b: 'x' }\n{ a: 1, b: 'x', c: 3 }\nP { a: 1, b: 'x' } false true\n"
    );
}

#[test]
fn lim4_collection_subclass_keeps_its_name_through_a_plain_base_parameter() {
    let source = r#"class M extends Map<string, number> { }
class S extends Set<number> { }
class A extends Array<number> { }
function pm(m: Map<string, number>): void { console.log(m); }
function ps(m: Set<number>): void { console.log(m); }
function pa(m: number[]): void { console.log(m); }
function main(): void {
  const m = new M(); m.set("a", 1);
  const s = new S(); s.add(2);
  const a = new A(); a.push(3);
  pm(m); ps(s); pa(a);
  pa([1, 2]);
  pm(new Map([["z", 9]]));
}
"#;
    assert_eq!(
        compile_and_run(source, "lim4_collection_subclass_plain_base_parameter"),
        "M(1) [Map] { 'a' => 1 }\nS(1) [Set] { 2 }\nA(1) [ 3 ]\n[ 1, 2 ]\nMap(1) { 'z' => 9 }\n"
    );
}
