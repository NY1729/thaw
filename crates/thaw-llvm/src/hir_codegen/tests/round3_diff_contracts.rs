// Round-3 diff contracts; expected output recorded from Node 22.

#[test]
fn array_from_length_source_with_any_mapper_param_stays_native() {
    let source = r#"function main(): void {
  const a = Array.from({ length: 9 }, (_: any, i: number) => i * 1000);
  console.log(a.length, a);
}"#;
    assert_eq!(
        compile_and_run(source, "r3_array_from_any_mapper"),
        "9 [\n     0, 1000, 2000,\n  3000, 4000, 5000,\n  6000, 7000, 8000\n]\n"
    );
}

#[test]
fn get_prototype_of_builtin_containers_is_the_shared_prototype() {
    let source = r#"function main(): void {
  console.log(Object.getPrototypeOf([]) === Array.prototype, Reflect.getPrototypeOf({}) === Object.prototype);
  console.log(Object.getPrototypeOf(new Map()) === Map.prototype);
}"#;
    assert_eq!(compile_and_run(source, "r3_get_prototype_of"), "true true\ntrue\n");
}

#[test]
fn group_by_and_regex_groups_have_null_prototypes() {
    let source = r#"function main(): void {
  console.log(Object.groupBy([1, 2, 3], (x: number) => (x % 2 ? "a" : "b")));
  console.log("x".match(/(?<n>x)/)!.groups);
}"#;
    assert_eq!(
        compile_and_run(source, "r3_null_proto"),
        "[Object: null prototype] { a: [ 1, 3 ], b: [ 2 ] }\n[Object: null prototype] { n: 'x' }\n"
    );
}
