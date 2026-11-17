// typeof of an element iterated out of an `any[]` (for-of, forEach, map, rest) reads the Json
// runtime type. Expected output from node 22.

#[test]
fn typeof_json_for_of_element_reads_runtime_type() {
    let source = r#"function main(): void {
  const forOf = (arr: any[]): void => {
    const parts: string[] = [];
    for (const x of arr) { parts.push(typeof x); }
    console.log(parts.join(' '));
  };
  forOf(['real', true, 1, null, undefined, { a: 1 }, [1]]);
}
"#;
    assert_eq!(
        compile_and_run(source, "typeof_json_for_of_element_reads_runtime_type"),
        "string boolean number object undefined object object\n"
    );
}

#[test]
fn typeof_json_for_each_element_reads_runtime_type() {
    let source = r#"function main(): void {
  const forEach = (arr: any[]): void => {
    const parts: string[] = [];
    arr.forEach((x: any): void => { parts.push(typeof x); });
    console.log(parts.join(' '));
  };
  forEach(['real', true, 1, null, undefined, { a: 1 }, [1]]);
}
"#;
    assert_eq!(
        compile_and_run(source, "typeof_json_for_each_element_reads_runtime_type"),
        "string boolean number object undefined object object\n"
    );
}

#[test]
fn typeof_json_map_element_reads_runtime_type() {
    let source = r#"function main(): void {
  const mapped = (arr: any[]): void => {
    const types: string[] = arr.map((x: any): string => typeof x);
    console.log(types.join(' '));
  };
  mapped(['real', true, 1, null, undefined, { a: 1 }, [1]]);
}
"#;
    assert_eq!(
        compile_and_run(source, "typeof_json_map_element_reads_runtime_type"),
        "string boolean number object undefined object object\n"
    );
}

#[test]
fn typeof_json_rest_element_reads_runtime_type() {
    let source = r#"function main(): void {
  const rest = (...a: any[]): void => {
    const parts: string[] = [];
    for (const x of a) { parts.push(typeof x); }
    console.log(parts.join(' '));
  };
  rest('real', true, 1, null, undefined, { a: 1 }, [1]);
}
"#;
    assert_eq!(
        compile_and_run(source, "typeof_json_rest_element_reads_runtime_type"),
        "string boolean number object undefined object object\n"
    );
}

#[test]
fn typeof_json_ternary_string_check_over_iteration() {
    let source = r#"function main(): void {
  const ternary = (arr: any[]): void => {
    const parts: string[] = [];
    for (const x of arr) { parts.push(typeof x === 'string' ? 'S' : 'O'); }
    console.log(parts.join(' '));
  };
  ternary(['real', true, 1, null, undefined, { a: 1 }, [1]]);
}
"#;
    assert_eq!(
        compile_and_run(source, "typeof_json_ternary_string_check_over_iteration"),
        "S O O O O O O\n"
    );
}

#[test]
fn typeof_json_holey_parameter_iteration() {
    let source = r#"function main(): void {
  const holey = (arr: any[]): void => {
    const parts: string[] = [];
    for (const x of arr) { parts.push(typeof x); }
    console.log(parts.join(' '));
  };
  holey([1, , 3]);
}
"#;
    assert_eq!(
        compile_and_run(source, "typeof_json_holey_parameter_iteration"),
        "number undefined number\n"
    );
}

#[test]
fn console_union_optional_json_member_prints_ternary_over_rest_for_each() {
    let source = r#"function main(): void {
  const printer = (...a: any[]): void => {
    a.forEach((x: any): void => {
      console.log(typeof x === 'string' ? x : JSON.stringify(x));
    });
  };
  printer('real', true);
}
"#;
    assert_eq!(
        compile_and_run(source, "console_union_optional_json_member_prints_ternary_over_rest_for_each"),
        "real\ntrue\n"
    );
}
