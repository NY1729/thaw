#[test]
fn preserves_local_function_overloads_while_renaming_modules() {
    let dir = std::env::temp_dir().join(format!("thaw overload module {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = r#"
        function label(value: number): string;
        function label(value: string): string;
        function label(value: number | string): string {
            return typeof value === "number" ? "n:" + String(value) : "s:" + value;
        }
        function main(): void { console.log(label(4), label("x")); }
    "#;
    std::fs::write(&entry, source).unwrap();
    let module = module_graph::bundle(
        &entry,
        source,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
    )
    .unwrap();
    thaw_hir::lower_module(&module).unwrap();
}

#[test]
fn create_require_literal_calls_are_external_dependencies() {
    let dir = std::env::temp_dir().join(format!("thaw create require {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url); const path = require('node:path');";
    std::fs::write(&entry, source).unwrap();
    let dependencies = module_graph::external_specifiers(&entry, source).unwrap();
    assert_eq!(
        dependencies
            .iter()
            .map(|(specifier, _)| specifier.as_str())
            .collect::<Vec<_>>(),
        vec!["node:module", "node:path"]
    );
}

#[test]
fn sourced_reexport_does_not_fall_back_to_a_same_named_local() {
    let dir = std::env::temp_dir().join(format!("thaw-reexport-source-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("other.ts"), "export const unrelated: number = 1;").unwrap();
    let entry = dir.join("main.ts");
    let source = "const value: number = 2; export { value } from './other';";
    std::fs::write(&entry, source).unwrap();
    let error = module_graph::bundle(&entry, source, &Default::default(), &Default::default(), &Default::default(), &Default::default()).unwrap_err();
    assert!(error.contains("cannot export unknown name `value`"), "{error}");
    let local = "const value: number = 2; export { value };";
    std::fs::write(&entry, local).unwrap();
    assert!(module_graph::bundle(&entry, local, &Default::default(), &Default::default(), &Default::default(), &Default::default()).is_ok());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn type_only_imports_keep_interfaces_without_running_dependency_statements() {
    let dir = std::env::temp_dir().join(format!("thaw-type-only-module-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("types.ts"),
        "export interface User { name: string; } export class Box { name: string = 'unused'; static { console.log('class static ran'); } } export enum Kind { One = 1 } console.log('type module ran');").unwrap();
    let entry = dir.join("main.ts");
    let source = "import type { User, Box, Kind } from './types'; function accept(box: Box, kind: Kind): void {} function main(): void { const user: User = { name: 'Ada' }; console.log(user.name); }";
    std::fs::write(&entry, source).unwrap();
    let module = module_graph::bundle(&entry, source, &Default::default(), &Default::default(), &Default::default(), &Default::default()).unwrap();
    thaw_hir::lower_module(&module).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout), "Ada\n");
    let named_type_source = "import { type User } from './types'; function main(): void { const user: User = { name: 'Grace' }; console.log(user.name); }";
    std::fs::write(&entry, named_type_source).unwrap();
    let named_type_module = module_graph::bundle(&entry, named_type_source, &Default::default(), &Default::default(), &Default::default(), &Default::default()).unwrap();
    thaw_hir::lower_module(&named_type_module).unwrap();
    let named_output = dir.join("named_app");
    build(&entry, &named_output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let named_result = Command::new(&named_output).output().unwrap();
    assert!(named_result.status.success());
    assert_eq!(String::from_utf8_lossy(&named_result.stdout), "Grace\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn type_only_external_packages_are_resolved_without_runtime_loading() {
    let dir = std::env::temp_dir().join(format!("thaw-type-only-external-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("types.ts"), "import type { Hidden } from 'types-only'; export type Alias = Hidden;").unwrap();
    let entry = dir.join("main.ts");
    let source = "import type { Alias } from './types'; import { value } from 'runtime-package'; function main(): void { console.log(value); }";
    std::fs::write(&entry, source).unwrap();
    let all = module_graph::external_specifiers(&entry, source).unwrap();
    assert!(all.iter().any(|(name, _)| name == "types-only"));
    assert!(all.iter().any(|(name, _)| name == "runtime-package"));
    let runtime = module_graph::external_runtime_specifiers(&entry, source).unwrap();
    assert!(!runtime.contains("types-only"));
    assert!(runtime.contains("runtime-package"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn type_only_module_keeps_link_features_for_retained_function_bodies() {
    let dir = std::env::temp_dir().join(format!("thaw-type-only-features-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("types.ts"),
        "export interface Shape { value: number; } export function unused(): void { const locale = new Intl.Locale('en'); console.log(locale); }").unwrap();
    let entry = dir.join("main.ts");
    let source = "import type { Shape } from './types'; function main(): void { const shape: Shape = { value: 1 }; console.log(shape.value); }";
    std::fs::write(&entry, source).unwrap();
    assert!(module_graph::runtime_features(&entry, source).unwrap().contains("intl"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn type_only_registry_package_does_not_load_its_bundle() {
    let dir = std::env::temp_dir().join(format!("thaw-type-registry-{}", std::process::id()));
    let registry = dir.join("registry");
    let package = registry.join("modules").join("types-only");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), "export interface Shape { value: number; }\nexport function f(): number;\nexport const value: number;\nexport class Client { read(): number; }\n").unwrap();
    std::fs::write(package.join("bundle.js"), "throw new Error('type-only bundle ran');\n").unwrap();
    std::fs::write(dir.join("types.ts"), "import { f, value } from 'types-only'; export interface LocalShape { value: number; } export function unused(): number { return f() + value; }").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, "import type { Shape, Client } from 'types-only'; import type { LocalShape } from './types'; function accept(value: Shape, client: Client, local: LocalShape): void {} function main(): void { console.log('ok'); }").unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn preserves_interface_declarations_for_typescript_merging() {
    let dir = std::env::temp_dir().join(format!("thaw interface merge {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = r#"interface User { name: string }
        interface User { age: number }
        function main(): void {
            const user: User = { name: "A", age: 1 };
            console.log(user.name, user.age);
        }"#;
    std::fs::write(&entry, source).unwrap();
    let module = module_graph::bundle(
        &entry,
        source,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
    )
    .unwrap();
    thaw_hir::lower_module(&module).unwrap();
}

#[test]
fn builds_relative_typescript_module_graph_with_generics_and_aliases() {
    let dir = std::env::temp_dir().join(format!("thaw cli user modules {}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/pair.ts"),
        r#"
                export interface Pair<T, U> { first: T; second: U; }
                export function makePair<T, U>(first: T, second: U): Pair<T, U> {
                    return { first: first, second: second };
                }
                export function chooseFirst<T, U>(first: T, second: U): T {
                    return first;
                }
            "#,
    )
    .unwrap();
    std::fs::write(
            dir.join("lib/values.ts"),
            "export function value(): number { return 40; }\nexport default function(): number { return 2; }\n",
        )
        .unwrap();
    std::fs::write(
            dir.join("lib/index.ts"),
            "export { makePair, chooseFirst } from './pair';\nexport { value, default as offset } from './values';\nexport * as values from './values';\n",
        )
        .unwrap();
    std::fs::write(
        dir.join("lib/other.ts"),
        "export function value(): number { return 2; }\n",
    )
    .unwrap();
    std::fs::write(
            dir.join("lib/expression.ts"),
            "function offset(): number { return 2; }\nexport function moduleUrl(): string { return import.meta.url; }\nexport function moduleFilename(): string { return import.meta.filename; }\nexport function moduleDirname(): string { return import.meta.dirname; }\nexport function moduleMain(): boolean { return import.meta.main; }\nexport function resolvedUrl(): string { return import.meta.resolve((`../${'data '}` as string) + 'file.json?raw#part'); }\nexport default offset;\n",
        )
        .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            r#"
                import { makePair, chooseFirst as first, value, values, offset as reexportedOffset } from "./lib";
                import offset, { value as sameValue } from "./lib/values";
                import { value as otherValue } from "./lib/other";
                import expressionOffset, { moduleUrl, moduleFilename, moduleDirname, moduleMain, resolvedUrl } from "./lib/expression";
                function main(): void {
                    const pair = makePair(value(), "ok");
                    console.log(pair.first + otherValue());
                    console.log(first("selected", sameValue() + offset()));
                    console.log(values.value() + values.default());
                    console.log(value() + reexportedOffset());
                    console.log(value() + expressionOffset());
                    console.log(moduleUrl());
                    console.log(moduleFilename());
                    console.log(moduleDirname());
                    console.log(moduleMain());
                    console.log(import.meta.main);
                    console.log(resolvedUrl());
                }
            "#,
        )
        .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!(
                "42\nselected\n42\n42\n42\nfile://{}\n{}\n{}\nfalse\ntrue\nfile://{}/data%20file.json?raw#part\n",
                dir.join("lib/expression.ts")
                    .display()
                    .to_string()
                    .replace(' ', "%20"),
                dir.join("lib/expression.ts").display(),
                dir.join("lib").display(),
                dir.display().to_string().replace(' ', "%20")
            )
        );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_and_runs_a_local_named_export_list_with_an_alias() {
    // `export { a, b as c } from '...'` (a re-export) is already covered
    // above; this is the bare form with no `from`, capturing multiple
    // same-file declarations -- including an aliasing rename -- in one
    // statement rather than one `export` per declaration.
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-local-named-export-list-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("lib.ts"),
        r#"
                const one = 1;
                function two(): number {
                    return 2;
                }
                export { one, two as second };
            "#,
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import { one, second } from "./lib";
                function main(): void {
                    console.log(one + second());
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_and_runs_classes_across_user_modules() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-user-module-classes-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("base.ts"),
        r#"
                export const Base = class Base {
                    constructor(public value: number) {}
                    #answer(): number { return this.value; }
                    answer(): number { return this.#answer(); }
                };
            "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("derived.ts"),
        r#"
                import { Base } from "./base";
                export default class Derived extends Base {
                    constructor(value: number, public label: string) { super(value); }
                }
            "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("label.ts"),
        r#"
                export default class {
                    constructor(public text: string) {}
                }
            "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("index.ts"),
        r#"
                export { default as Derived } from "./derived";
                export { default as Label } from "./label";
                export { Box as LeftBox } from "./left";
                export { Box as RightBox } from "./right";
            "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("left.ts"),
        "export class Box { constructor(public value: number) {} }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("right.ts"),
        "export class Box { constructor(public value: string) {} }\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import { Derived, Label, LeftBox, RightBox } from "./index";
                import * as models from "./index";
                function read(value: Derived): number { return value.answer(); }
                function main(): void {
                    const value = new Derived(42, "ready");
                    const label = new Label("module class");
                    const namespaced = new models.Derived(7, "namespace");
                    const left = new LeftBox(8);
                    const right = new RightBox("separate");
                    console.log(read(value));
                    console.log(value.label);
                    console.log(label.text);
                    console.log(namespaced.answer());
                    console.log(namespaced.label);
                    console.log(left.value);
                    console.log(right.value);
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "42\nready\nmodule class\n7\nnamespace\n8\nseparate\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_generic_class_specializations_across_user_modules() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-user-module-generic-classes-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("models.ts"),
        r#"
                export class Box<T> {
                    static count: number = 0;
                    static { Box.count += 1; }
                    constructor(public value: T) { Box.count += 1; }
                    get(): T { return this.value; }
                    convert<U>(value: U): U { return value; }
                    static identity<U>(value: U): U { return value; }
                }
                export class Pair<T, U> {
                    constructor(public first: T, public second: U) {}
                }
                export class Holder<T> {
                    constructor(public value: T) {}
                }
                export class NumberBox extends Box<number> {
                    double(): number { return this.value * 2; }
                }
                export function makeNumber(): number { return 6; }
            "#,
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            r#"
                import { Box, Pair, Holder, NumberBox, makeNumber } from "./models";
                function read(value: Box<number>): number { return value.get(); }
                function main(): void {
                    const first = new Box<number>(40);
                    const duplicate = new Box<number>(2);
                    const text = new Box<string>("module");
                    const pair = new Pair<string, number>(text.get(), read(first) + duplicate.get());
                    const nested = new Holder<Box<number>>(first);
                    const inferredNested = new Holder(first);
                    const nestedBox = nested.value;
                    const inferredNestedBox = inferredNested.value;
                    const derived = new NumberBox(21);
                    const fromCall = new Box(makeNumber());
                    console.log(pair.first);
                    console.log(pair.second);
                    console.log(nestedBox.get());
                    console.log(inferredNestedBox.get());
                    console.log(first.convert("method"));
                    console.log(Box.identity(true));
                    console.log(fromCall.get());
                    console.log(Box.count);
                    console.log(derived.double());
                    console.log(derived.get());
                }
            "#,
        )
        .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "module\n42\n40\n40\nmethod\ntrue\n6\n6\n42\n21\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_multifile_top_level_bindings_into_one_executable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-top-level-bindings-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("values.ts"),
        r#"
                export const base = 40;
                export const answer = makeAnswer();
                export let counter = answer;
                function makeAnswer(): number { return base + 2; }
                export function next(): number {
                    counter = counter + 1;
                    return counter;
                }
            "#,
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import { answer, next } from "./values";
                const label = "answer";
                const record = { answer };
                function local(answer: number): number {
                    answer = answer + 1;
                    return answer;
                }
                function main(): void {
                    console.log(label);
                    console.log(record.answer);
                    console.log(answer);
                    console.log(local(1));
                    console.log(next());
                    console.log(next());
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "answer\n42\n42\n2\n43\n44\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn compiles_general_default_export_expressions_once() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-default-export-expression-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("value.ts"),
        r#"
                let calls = 0;
                function compute(): number {
                    calls++;
                    return 40 + 2;
                }
                export function getCalls(): number { return calls; }
                export default { answer: compute(), label: "ready" };
                console.log("value init");
            "#,
    )
    .unwrap();
    std::fs::write(
            dir.join("left.ts"),
            "import value from './value'; export const leftAtInit = value.answer + 1; console.log('left init');",
        )
        .unwrap();
    std::fs::write(
            dir.join("right.ts"),
            "import value from './value'; export const rightAtInit = value.answer + 2; console.log('right init');",
        )
        .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import value, { getCalls } from "./value";
                import { leftAtInit } from "./left";
                import { rightAtInit } from "./right";
                console.log("entry init");
                function main(): void {
                    console.log(value.label);
                    console.log(value.answer);
                    console.log(value.answer);
                    console.log(leftAtInit);
                    console.log(rightAtInit);
                    console.log(getCalls());
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "value init\nleft init\nright init\nentry init\nready\n42\n42\n43\n44\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn top_level_exception_skips_main_and_fails_the_process() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-top-level-exception-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                console.log("before failure");
                throw "module initialization failed";
                function main(): void { console.log("main must not run"); }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "before failure\n");
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "Uncaught: module initialization failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// An uncaught `new Error(...)`/subclass throw used to print as the
/// illegible "Errorboom" -- `thaw_runtime_report_uncaught`
/// (thaw-runtime) printed the exception channel's raw tagged string
/// directly, whose `\u{1}Error\u{1}boom`-style control-byte markers
/// aren't visible in a terminal, so the real separator real Node
/// always includes ("Error: boom") was simply missing. A plain
/// untagged string throw (the sibling test above,
/// `top_level_exception_skips_main_and_fails_the_process`) was
/// already correct and stays that way -- real Node prints a non-Error
/// thrown value completely unchanged, no "Error:" prefix at all.
#[test]
fn top_level_error_throw_reports_name_and_message() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-top-level-error-throw-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                class MyError extends Error {
                    constructor(message: string) {
                        super(message);
                        this.name = "MyError";
                    }
                }
                function main(): void {
                    throw new MyError("custom oops");
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "Uncaught: MyError: custom oops\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Literal local imports resolve at build time while their module initializer
/// runs only from the import Promise continuation. Computed specifiers remain
/// unsupported by the ahead-of-time resolver.
#[test]
fn dynamic_import_of_a_literal_local_path_resolves_named_and_default_exports() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-dynamic-import-literal-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("other.ts"),
        r#"
                export function greet(): string { return "hi"; }
                export function farewell(): string { return "bye"; }
                export default function (): string { return "default export"; }
            "#,
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                async function main(): Promise<void> {
                    const mod = await import("./other");
                    console.log(mod.greet());
                    console.log(mod.farewell());
                    console.log(mod.default());
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "hi\nbye\ndefault export\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_import_initializes_once_and_reads_live_exports() {
    let dir = std::env::temp_dir().join(format!("thaw-lazy-local-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("later.ts"),
        "console.log('loaded'); export let count: number = 1; export function increment(): void { count++; }").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry,
        "async function main(): Promise<void> { console.log('before'); const first = await import('./later'); first.increment(); const second = await import('./later'); console.log(first === second, first.count, second.count); }").unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "before\nloaded\ntrue 2 2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_import_caches_initialization_failure() {
    let dir = std::env::temp_dir().join(format!("thaw-lazy-failure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("later.ts"),
        "console.log('attempt'); throw new Error('boom'); export const value: number = 1;").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry,
        "async function main(): Promise<void> { try { await import('./later'); } catch (error) { console.log('rejected'); } try { await import('./later'); } catch (error) { console.log('rejected'); } }").unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "attempt\nrejected\nrejected\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_import_failure_cache_survives_lambda_arena_reset() {
    let dir = std::env::temp_dir().join(format!("thaw-lazy-lambda-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("later.ts"),
        "throw new Error('cached failure'); export const value: number = 1;").unwrap();
    let entry = dir.join("handler.ts");
    std::fs::write(&entry,
        "async function handler(event: Json): Promise<Json> { try { await import('./later'); } catch (error) { console.log(error.message); } return event; }").unwrap();
    let output = dir.join("bootstrap");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (sender, receiver) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = connection.read(&mut request).unwrap();
            let event = format!("{{\"invocation\":{index}}}");
            connection.write_all(format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: lazy-{index}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{event}",
                event.len(),
            ).as_bytes()).unwrap();
            drop(connection);
            let (mut connection, _) = listener.accept().unwrap();
            let mut posted = Vec::new();
            connection.read_to_end(&mut posted).unwrap();
            sender.send(String::from_utf8_lossy(&posted).into_owned()).unwrap();
            connection.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        }
    });
    let mut child = Command::new(&output)
        .env("AWS_LAMBDA_RUNTIME_API", address)
        .stdout(Stdio::piped())
        .spawn().unwrap();
    for index in 0..2 {
        let posted = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(posted.starts_with(&format!(
            "POST /2018-06-01/runtime/invocation/lazy-{index}/response"
        )), "{posted}");
    }
    server.join().unwrap();
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "cached failure\ncached failure\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_external_import_defers_package_script() {
    let dir = std::env::temp_dir().join(format!("thaw-lazy-package-{}", std::process::id()));
    let package = dir.join("registry/modules/later");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), "export function value(): number;").unwrap();
    std::fs::write(package.join("bundle.js"),
        "globalThis.__later_count = (globalThis.__later_count || 0) + 1; module.exports.value = function() { return globalThis.__later_count; };").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry,
        "async function main(): Promise<void> { console.log('before'); const first = await import('later'); const second = await import('later'); console.log(first === second, second.value()); }").unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "before\ntrue 1\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_import_initializes_static_dependencies_only_on_first_call() {
    let dir = std::env::temp_dir().join(format!("thaw-lazy-dependency-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("base.ts"), "console.log('base'); export const value: number = 7;").unwrap();
    std::fs::write(dir.join("later.ts"), "import { value } from './base'; console.log('later'); export const result: number = value;").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry,
        "async function main(): Promise<void> { console.log('before'); const first = await import('./later'); const second = await import('./later'); console.log(first.result, second.result); }").unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "before\nbase\nlater\n7 7\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn side_effect_only_external_dynamic_import_is_deferred() {
    let dir = std::env::temp_dir().join(format!("thaw-lazy-side-effect-{}", std::process::id()));
    let package = dir.join("registry/modules/side-effect");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), "export interface Marker { value: number; }").unwrap();
    std::fs::write(package.join("bundle.js"), "console.log('package loaded');").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry,
        "async function main(): Promise<void> { console.log('before'); await import('side-effect'); await import('side-effect'); console.log('after'); }").unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "before\npackage loaded\nafter\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_back_edge_to_a_static_dependency_keeps_one_initialization() {
    let dir = std::env::temp_dir().join(format!("thaw-lazy-cycle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("later.ts"),
        "import { base } from './main'; console.log('later'); export const result: number = base + 1;").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry,
        "export const base: number = 7; async function main(): Promise<void> { console.log('before'); const later = await import('./later'); console.log(later.result); }").unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "before\nlater\n8\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn static_import_cycle_is_rejected_before_emitting_uninitialized_exports() {
    let dir = std::env::temp_dir().join(format!("thaw-static-cycle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("other.ts"), "import { left } from './main'; export const right: number = left + 1;").unwrap();
    let entry = dir.join("main.ts");
    let source = "import { right } from './other'; export const left: number = right + 1; function main(): void {}";
    std::fs::write(&entry, source).unwrap();
    let error = module_graph::bundle(&entry, source, &Default::default(), &Default::default(), &Default::default(), &Default::default()).unwrap_err();
    assert!(error.contains("cyclic static user-module import"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn type_only_cycle_keeps_both_interface_declarations_without_startup_effects() {
    let dir = std::env::temp_dir().join(format!("thaw-type-cycle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("other.ts"),
        "import type { Left } from './main'; export interface Right { value: number; } export function accept(left: Left): void {} console.log('other initialized');").unwrap();
    let entry = dir.join("main.ts");
    let source = "import type { Right } from './other'; export interface Left { value: number; } function accept(right: Right): void {} function main(): void { console.log('ready'); }";
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ready\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_import_of_a_non_literal_specifier_is_still_rejected() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-dynamic-import-non-literal-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                async function main(): Promise<void> {
                    const name = "other";
                    const mod = await import(name);
                    console.log(mod);
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    let error = build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap_err();
    assert!(error.contains("import calls not supported"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn exports_top_level_destructured_bindings() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-top-level-destructuring-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("values.ts"),
        r#"
                interface Config { fallback: number | undefined; }
                export const { answer, label: text } = { answer: 42, label: "ready" };
                export const [first, second] = [20, 22];
                export const { fallback = 42 }: Config = { fallback: undefined };
                export const [head, ...tail] = [20, 10, 12];
                export const { primary, ...metadata } = { primary: 42, label: "meta", code: 2 };
            "#,
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            r#"
                import { answer, text, first, second, fallback, head, tail, primary, metadata } from "./values";
                function main(): void {
                    console.log(text);
                    console.log(answer);
                    console.log(first + second);
                    console.log(fallback);
                    console.log(head + tail[0] + tail[1]);
                    console.log(metadata.label);
                    console.log(primary + metadata.code - 2);
                }
            "#,
        )
        .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success());
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "ready\n42\n42\n42\n42\nmeta\n42\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resolves_and_reports_star_export_ambiguity() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-star-export-ambiguity-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
            dir.join("left.ts"),
            "export function left(): number { return 1; } export function shared(): number { return 10; }",
        )
        .unwrap();
    std::fs::write(
            dir.join("right.ts"),
            "export function right(): number { return 2; } export function shared(): number { return 20; }",
        )
        .unwrap();
    std::fs::write(
        dir.join("index.ts"),
        "export * from './left'; export * from './right'; export { shared } from './left';",
    )
    .unwrap();
    std::fs::write(dir.join("barrel.ts"), "export * from './index';").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            "import { left, right, shared } from './barrel'; function main(): void { console.log(left() + right() + shared()); }",
        )
        .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout), "13\n");

    std::fs::write(
        dir.join("index.ts"),
        "export * from './left'; export * from './right';",
    )
    .unwrap();
    let source = std::fs::read_to_string(&entry).unwrap();
    let error = module_graph::bundle(
        &entry,
        &source,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
    )
    .unwrap_err();
    assert!(
        error.contains("ambiguous star export named `shared`"),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_relative_typescript_import_cycles_with_the_full_chain() {
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-user-module-cycle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.ts"),
        "import { b } from './b'; function main(): void { b(); }",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.ts"),
        "import { c } from './c'; export function b(): void { c(); }",
    )
    .unwrap();
    std::fs::write(
        dir.join("c.ts"),
        "import { b } from './b'; export function c(): void { b(); }",
    )
    .unwrap();
    let error = module_graph::bundle(
        &dir.join("main.ts"),
        &std::fs::read_to_string(dir.join("main.ts")).unwrap(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
    )
    .unwrap_err();
    assert!(error.contains("cyclic static user-module import"));
    assert!(error.contains("b.ts"));
    assert!(error.contains("c.ts"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_and_runs_a_multifile_async_json_lambda_handler() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-user-module-lambda-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("transform.ts"),
        r#"
                interface PendingEvent { value: Promise<Json>; }
                async function delayed(event: Json): Promise<Json> {
                    await sleep(1);
                    return event;
                }
                export async function transform(event: Json): Promise<Json> {
                    const chained: Promise<Json> = new Promise<Json>((resolve, reject) => {
                        resolve(event);
                    }).then(value => delayed(value));
                    const pending: PendingEvent = { value: chained };
                    return await pending.value;
                }
            "#,
    )
    .unwrap();
    let entry = dir.join("handler.ts");
    std::fs::write(
        &entry,
        r#"
                import { transform } from "./transform";
                async function handler(event: Json): Promise<Json> {
                    return await transform(event);
                }
            "#,
    )
    .unwrap();
    let output = dir.join("bootstrap");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = conn.read(&mut request).unwrap();
        let event = "{\"message\":\"module lambda\"}";
        conn.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: module-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    event.len(), event
                )
                .as_bytes(),
            )
            .unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        tx.send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let mut child = Command::new(&output)
        .env("AWS_LAMBDA_RUNTIME_API", addr)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let request = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("multifile Lambda handler did not post a response");
    server.join().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert!(request.starts_with("POST /2018-06-01/runtime/invocation/module-request/response"));
    assert!(request.ends_with("{\"message\":\"module lambda\"}"));
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn block_and_catch_bindings_do_not_shadow_outer_module_references() {
    let dir = std::env::temp_dir().join(format!("thaw lexical blocks {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(dir.join("helper.ts"), "export function getValue(): number { return 9; }").unwrap();
    std::fs::write(&input, r#"
        import { getValue } from './helper';
        import * as lib from './helper';
        const value: number = 7;
        function main(): void {
            if (true) { const value: number = 3; console.log(value); }
            console.log(value);
            for (let value: number = 0; value < 1; value++) { console.log(value); }
            console.log(value);
            const f = (): void => {
                { const value: number = 5; console.log(value); }
                console.log(value);
            };
            f();
            try { throw "caught"; } catch (value) { console.log(value); }
            console.log(value);
            { const getValue = (): number => 3; console.log(getValue()); }
            console.log(getValue());
            { const lib = { getValue: (): number => 4 }; console.log(lib.getValue()); }
            console.log(lib.getValue());
        }
    "#).unwrap();
    build(&input, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3\n7\n0\n7\n5\n7\ncaught\n7\n3\n9\n4\n9\n");
    std::fs::remove_dir_all(dir).unwrap();
}
