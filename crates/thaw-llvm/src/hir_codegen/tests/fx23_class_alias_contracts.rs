// Class alias construction: `const E = D` / `let E = D` (D a module class) makes `new E()` lower as `new D()`.
// A reassigned alias and a shadowed name keep their own bindings. Expected output from node 22.

#[test]
fn fx23_global_class_alias_construction_prints_the_class() {
    let source = r#"
class D { q = 1 }
const E = D;
function main(): void { console.log(new E()); }
"#;
    let output = compile_and_run(
        source,
        "fx23_global_class_alias_construction_prints_the_class",
    );
    assert_eq!(output, "D { q: 1 }\n");
}

#[test]
fn fx23_local_class_alias_construction_prints_the_class() {
    let source = r#"
class D { q = 1 }
function main(): void { const E = D; console.log(new E(), new E().q); }
"#;
    let output = compile_and_run(
        source,
        "fx23_local_class_alias_construction_prints_the_class",
    );
    assert_eq!(output, "D { q: 1 } 1\n");
}

#[test]
fn fx23_reassigned_let_alias_keeps_the_assigned_class() {
    let source = r#"
class D { q = 1 }
class F { q = 7 }
let E = D;
E = F;
function main(): void { console.log(new E().q); }
"#;
    let output = compile_and_run(source, "fx23_reassigned_let_alias_keeps_the_assigned_class");
    assert_eq!(output, "7\n");
}

#[test]
fn fx23_shadowed_alias_name_is_not_renamed() {
    let source = r#"
class D { q = 1 }
class F { q = 7 }
const E = D;
function g(E: any): number { return new E().q; }
function main(): void { console.log(g(F), new E().q); }
"#;
    let output = compile_and_run(source, "fx23_shadowed_alias_name_is_not_renamed");
    assert_eq!(output, "7 1\n");
}
