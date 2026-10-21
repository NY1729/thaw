// Wave NA: static builtin results written into `any`, nested method calls on a class reached
// through an `any` constructor, and `Object.hasOwn` on a shadowed inherited static field.
// Expected output from node 22.

#[test]
fn na_contract_static_builtin_results_assigned_to_any() {
    let source = r##"function main(): void {
  const a: any = Object.fromEntries([["a", 1], ["b", 2]]);
  console.log(JSON.stringify(a));
  const g: any = Object.groupBy([1, 2, 3, 4], (n: number) => (n % 2 === 0 ? "even" : "odd"));
  console.log(JSON.stringify(g));
  const m: any = Map.groupBy([1, 2, 3, 4], (n: number) => (n % 2 === 0 ? "even" : "odd"));
  console.log(m.size, JSON.stringify(m.get("even")));
}
"##;
    assert_eq!(
        compile_and_run(source, "na_contract_static_builtin_results_assigned_to_any"),
        "{\"a\":1,\"b\":2}\n{\"odd\":[1,3],\"even\":[2,4]}\n2 [2,4]\n"
    );
}

#[test]
fn na_contract_nested_method_call_on_class_built_through_any_constructor() {
    let source = r##"class Counter {
  count: number = 0;
  increment(): void { this.count = this.count + 1; }
  touch(): void { this.increment(); }
}
function main(): void {
  const value: any = new (Counter as any)();
  value.touch();
  value.touch();
  console.log(value.count);
  const C: any = Counter;
  const other: any = new C();
  other.increment();
  other.touch();
  console.log(other.count);
}
"##;
    assert_eq!(
        compile_and_run(source, "na_contract_nested_method_call_on_class_built_through_any_constructor"),
        "2\n2\n"
    );
}

#[test]
fn na_contract_has_own_sees_a_shadowed_inherited_static_field() {
    let source = r##"class Parent { static value: number = 1; static other: number = 5; }
class Child extends Parent { static own: number = 9; }
function main(): void {
  console.log(Object.hasOwn(Child, "value"), Object.hasOwn(Child, "own"));
  Child.value = 2;
  console.log(Parent.value, Child.value, Object.hasOwn(Child, "value"), Object.hasOwn(Child, "other"));
}
"##;
    assert_eq!(
        compile_and_run(source, "na_contract_has_own_sees_a_shadowed_inherited_static_field"),
        "false true\n1 2 true false\n"
    );
}
