
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn detects_dynamic_interpreters_in_elf_outputs() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-elf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app");
    let mut elf = vec![0_u8; 120];
    elf[..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2;
    elf[5] = 1;
    elf[32..40].copy_from_slice(&64_u64.to_le_bytes());
    elf[54..56].copy_from_slice(&56_u16.to_le_bytes());
    elf[56..58].copy_from_slice(&1_u16.to_le_bytes());
    elf[64..68].copy_from_slice(&3_u32.to_le_bytes());
    std::fs::write(&path, &elf).unwrap();
    assert!(elf_has_program_interpreter(&path).unwrap());

    elf[64..68].copy_from_slice(&1_u32.to_le_bytes());
    std::fs::write(&path, &elf).unwrap();
    assert!(!elf_has_program_interpreter(&path).unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn static_toolchain_check_is_actionable_or_complete() {
    if let Err(error) = ensure_static_system_libraries() {
        assert!(error.contains("--static"), "{error}");
        assert!(error.contains("glibc-static"), "{error}");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn builds_and_runs_a_fully_static_elf() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-static-elf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(&source, "function main(): void { console.log(42); }\n").unwrap();
    build_with_link_mode(
        &source,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    assert!(!elf_has_program_interpreter(&output).unwrap());
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["packages"], serde_json::json!([]));
    assert_eq!(manifest["quickjs"], false);
    assert_eq!(manifest["napi"], false);
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn static_build_rejects_dynamic_napi_addons() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-static-napi-{}", std::process::id()));
    let package = dir.join("registry/native-add");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function add(a: number, b: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("native.node"),
        b"not loaded during compilation",
    )
    .unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        "import { add } from \"native-add\"; function main(): void {}\n",
    )
    .unwrap();
    let error = build_with_link_mode(
        &source,
        &dir.join("app"),
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap_err();
    assert!(error.contains("N-API addon"), "{error}");
    assert!(error.contains("native.a"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn fully_static_binary_runs_in_an_isolated_container_when_enabled() {
    if std::env::var("THAW_RUN_CONTAINER_INTEGRATION").as_deref() != Ok("1")
        || ensure_static_system_libraries().is_err()
    {
        return;
    }
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-static-container-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(&source, "function main(): void { console.log(42); }\n").unwrap();
    build_with_link_mode(
        &source,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    let result = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "--security-opt",
            "label=disable",
            "-v",
        ])
        .arg(format!("{}:/app:ro", output.display()))
        .args(["registry.fedoraproject.org/fedora:41", "/app"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
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
    )
    .unwrap_err();
    assert!(error.contains("cyclic user-module import"));
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
fn bare_imports_automatically_resolve_registry_packages() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-bare-imports-{}", std::process::id()));
    let registry = dir.join("modules");
    std::fs::create_dir_all(registry.join("math-kit")).unwrap();
    std::fs::write(
            registry.join("math-kit/package.d.ts"),
            "export interface Point { x: number; y: number; }\nexport declare function add(a: number, b: number): number;\nexport declare function sub(a: number, b: number): number;\nexport declare function greet(name: string): string;\nexport declare function negate(value: boolean): boolean;\nexport declare function echo(value: Json): Json;\nexport declare function sum(values: number[]): number;\nexport declare function reverse(values: number[]): number[];\nexport declare function shift(point: Point): Point;\nexport declare function fail(): number;\n",
        )
        .unwrap();
    std::fs::write(
            registry.join("math-kit/bundle.js"),
            "module.exports = { add: function(a,b){ return a+b; }, sub: function(a,b){ return a-b; }, greet: function(name){ return 'hello ' + name; }, negate: function(value){ return !value; }, echo: function(value){ return value; }, sum: function(values){ return values.reduce(function(a,b){ return a+b; }, 0); }, reverse: function(values){ return values.reverse(); }, shift: function(point){ return { x: point.x + 1, y: point.y + 2 }; }, fail: function(){ throw new Error('typed dynamic failed'); } };\n",
        )
        .unwrap();
    std::fs::create_dir_all(registry.join("math-kit/subpaths/advanced")).unwrap();
    std::fs::write(
        registry.join("math-kit/subpaths/advanced/package.d.ts"),
        "export declare function square(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        registry.join("math-kit/subpaths/advanced/bundle.js"),
        "module.exports = { square: function(value){ return value * value; } };\n",
    )
    .unwrap();
    std::fs::create_dir_all(registry.join("twice")).unwrap();
    std::fs::write(
        registry.join("twice/package.d.ts"),
        "export default function twice(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        registry.join("twice/bundle.js"),
        "module.exports = function(value){ return value * 2; };\n",
    )
    .unwrap();

    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import { add, greet, negate, echo, sum, reverse, shift, fail } from "math-kit";
                import * as math from "math-kit";
                import { square } from "math-kit/advanced";
                import twice from "twice";
                function main(): void {
                    const sum: number = add(10, 11);
                    const difference: number = math.sub(13, 2);
                    console.log(twice(21));
                    console.log(sum + difference);
                    console.log(square(7));
                    console.log(greet("thaw"));
                    console.log(negate(false));
                    console.log(String(echo(JSON.parse("{\"ok\":true}")).ok));
                    console.log(sum([10, 20, 12]));
                    const reversed = reverse([1, 2, 3]);
                    console.log(reversed[0]);
                    const point = shift({ x: 3, y: 4 });
                    console.log(point.x * 10 + point.y);
                    console.log(import.meta.resolve("math-kit"));
                    console.log(import.meta.resolve("math-kit/advanced?raw"));
                    try {
                        const ignored = fail();
                    } catch (error) {
                        console.log(error);
                    }
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!(
                "42\n32\n49\nhello thaw\ntrue\ntrue\n42\n3\n46\nfile://{}\nfile://{}?raw\n`math-kit::fail` threw: typed dynamic failed\n",
                registry.join("math-kit/bundle.js").display(),
                registry
                    .join("math-kit/subpaths/advanced/bundle.js")
                    .display()
            )
        );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn import_meta_resolution_prefers_the_selected_registry_backend() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-import-meta-backend-{}",
        std::process::id()
    ));
    let package = dir.join("pkg");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("bundle.js"), "module.exports = {};").unwrap();
    std::fs::write(package.join("native.node"), []).unwrap();
    let resolutions =
        registry_import_meta_resolutions(&dir, &["pkg".to_string(), "node:fs".to_string()]);
    assert_eq!(
        resolutions.get("pkg"),
        Some(&module_graph::file_url(&package.join("native.node")))
    );
    assert_eq!(
        resolutions.get("node:fs").map(String::as_str),
        Some("node:fs")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_bundle_uses_timers_microtasks_and_text_encoding() {
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-platform-globals-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("platform-work");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
            package.join("package.d.ts"),
            "export interface PendingValue { (): void; }\nexport declare function exercise(seed: number): PendingValue;\n",
        )
        .unwrap();
    std::fs::write(
            package.join("bundle.js"),
            r#"module.exports = { exercise: function() {
                return new Promise(resolve => {
                    const events = [];
                    const cancelled = setTimeout(() => events.push('cancelled'), 0);
                    clearTimeout(cancelled);
                    const cancelledImmediate = setImmediate(() => events.push('cancelled-immediate'));
                    clearImmediate(cancelledImmediate);
                    process.nextTick(value => events.push(value), 'nextTick');
                    queueMicrotask(() => events.push('microtask'));
                    setImmediate(value => events.push(value), 'immediate');
                    let ticks = 0;
                    const interval = setInterval(() => {
                        ticks++;
                        if (ticks === 2) {
                            clearInterval(interval);
                            const bytes = new TextEncoder().encode('雪');
                            const text = new TextDecoder().decode(bytes);
                            const original = { nested: { value: 1 } };
                            const copied = structuredClone(original);
                            copied.nested.value = 2;
                            const controller = new AbortController();
                            controller.abort('stopped');
                            const combined = AbortSignal.any([controller.signal]);
                            setTimeout(() => resolve(events.join(',') + ':' + ticks + ':' + text + ':' + btoa('hi') + ':' + atob('aGk=') + ':' + (performance.now() >= 0) + ':' + original.nested.value + ':' + copied.nested.value + ':' + combined.aborted + ':' + combined.reason), 0);
                        }
                    }, 1);
                });
            } };"#,
        )
        .unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { exercise } from "platform-work";
                function main(): void {
                    const pending: JsValue = exercise(0);
                    console.log(String(readDynamicValue(pending)));
                    releaseDynamicValue(pending);
                }"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "nextTick,microtask,immediate:2:雪:aGk=:hi:true:1:2:true:stopped\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn imports_supported_node_builtin_modules() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-imports-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            r#"
                import * as path from "node:path";
                import { inspect, format } from "node:util";
                import { cwd } from "node:process";
                import { byteLength } from "node:buffer";
                import * as os from "node:os";
                import * as querystring from "node:querystring";
                import { EventEmitter } from "node:events";
                import { strictEqual } from "node:assert/strict";
                import { StringDecoder } from "node:string_decoder";
                import { isatty } from "node:tty";
                import { pathToFileURL, fileURLToPath, urlToHttpOptions } from "node:url";
                function main(): void {
                    console.log(String(path.join(JSON.parse("[\"a\",\"b\"]"))));
                    console.log(String(path.extname(JSON.parse("[\"archive.tar.gz\"]"))));
                    console.log(String(path.relative(JSON.parse("[\"/a/b\",\"/a/c/d\"]"))));
                    console.log(String(inspect(JSON.parse("[42]"))));
                    console.log(String(format(JSON.parse("[\"%s:%d\",\"value\",4]"))));
                    console.log(String(cwd(JSON.parse("[]"))));
                    console.log(Number(byteLength(JSON.parse("[\"thaw\"]"))));
                    console.log(String(os.arch(JSON.parse("[]"))) + ":" + String(os.platform(JSON.parse("[]"))) + ":" + String(os.type(JSON.parse("[]"))) + ":" + String(os.tmpdir(JSON.parse("[]"))));
                    console.log(Boolean(isatty(JSON.parse("[1]"))));
                    console.log(String(querystring.stringify(JSON.parse("[{\"a\":[1,2],\"space\":\"two words\"}]"))));
                    console.log(String(querystring.parse(JSON.parse("[\"a=1&a=2&space=two+words\"]"))));
                    console.log(String(EventEmitter(JSON.parse("[]"))));
                    console.log(String(pathToFileURL(JSON.parse("[\"/tmp/a b\"]"))));
                    console.log(String(fileURLToPath(JSON.parse("[\"file:///tmp/a%20b\"]"))));
                    console.log(String(urlToHttpOptions(JSON.parse("[\"https://user:pass@example.test:8443/a?b=1#c\"]"))));
                }
            "#,
        )
        .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected_tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!("a/b\n.gz\n../c/d\n42\nvalue:4\n/\n4\nx64:linux:Linux:{expected_tmpdir}\nfalse\na=1&a=2&space=two%20words\n{{\"a\":[\"1\",\"2\"],\"space\":\"two words\"}}\n{{\"_events\":{{}}}}\nfile:///tmp/a%20b\n/tmp/a b\n{{\"auth\":\"user:pass\",\"hash\":\"#c\",\"hostname\":\"example.test\",\"href\":\"https://user:pass@example.test:8443/a?b=1#c\",\"path\":\"/a?b=1\",\"pathname\":\"/a\",\"port\":8443,\"protocol\":\"https:\",\"search\":\"?b=1\"}}\n")
        );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_fs_reads_and_writes_real_files_in_a_static_binary() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-fs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data_dir = dir.join("data");
    let data_file = data_dir.join("message.txt");
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        format!(
            r#"
                    import {{ existsSync, readFileSync, writeFileSync, mkdirSync }} from "node:fs";
                    function main(): void {{
                        console.log(mkdirSync("{}"));
                        console.log(writeFileSync("{}", "hello from thaw"));
                        console.log(existsSync("{}"));
                        console.log(readFileSync("{}", "utf8"));
                    }}
                "#,
            data_dir.display(),
            data_file.display(),
            data_file.display(),
            data_file.display()
        ),
    )
    .unwrap();
    let output = dir.join("app");
    build_with_link_mode(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\ntrue\ntrue\nhello from thaw\n"
    );
    assert_eq!(
        std::fs::read_to_string(data_file).unwrap(),
        "hello from thaw"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_http_serves_a_real_request_from_a_static_binary() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let occupied_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let occupied_port = occupied_listener.local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-node-http-{}-{}",
        std::process::id(),
        port
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            format!(
                r#"
                    import {{ createServer }} from "node:http";
                    function main(): void {{
                        const prefix: string = "hello";
                        let requests: number = 0;
                        const server = createServer(
                            (
                                request: {{ method: string; url: string }},
                                response: {{
                                    statusCode: number;
                                    setHeader: (name: string, value: string) => boolean;
                                    write: (chunk: string) => boolean;
                                    end: (chunk: string) => boolean;
                                }}
                            ): boolean => {{
                                requests = requests + 1;
                                response.statusCode = 201;
                                response.setHeader("X-Thaw", request.method);
                                response.write(prefix);
                                return response.end(request.url);
                            }}
                        );
                        const target: string = server.listenMany({}, 2);
                        console.log(target);
                        console.log(requests);
                        console.log(server.close());
                        console.log(server.close());
                        server.on("listening", (): void => {{
                            console.log("event:listening");
                        }});
                        server.on("close", (): void => {{
                            console.log("event:close");
                        }});
                        server.on("error", (error: {{ message: string; code: string; syscall: string; address: string; port: number }}): void => {{
                            console.log(error.code);
                        }});
                        server.listen({}, (): void => {{
                            console.log("listening");
                        }});
                        console.log(server.close((): void => {{
                            console.log("closed");
                        }}));
                        server.listen(70000);
                        server.listen({});
                    }}
                "#,
                port, port, occupied_port
            ),
        )
        .unwrap();
    let executable = dir.join("app");
    build_with_link_mode(
        &entry,
        &executable,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    assert!(!elf_has_program_interpreter(&executable).unwrap());

    let child = Command::new(&executable)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = |target: &str| {
        let mut stream = (0..200)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                    None
                }
            })
            .expect("compiled HTTP server did not start listening");
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let response = request("/health");
    let second_response = request("/ready");
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(response.starts_with("HTTP/1.1 201 Created\r\n"));
    assert!(response.contains("X-Thaw: GET\r\n"));
    assert!(response.ends_with("hello/health"));
    assert!(second_response.ends_with("hello/ready"));
    assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            "/ready\n2\ntrue\nfalse\ntrue\nevent:listening\nlistening\nERR_SOCKET_BAD_PORT\nEADDRINUSE\nevent:close\nclosed\n"
        );

    std::fs::write(
            &entry,
            r#"import { createServer } from "node:http";
            function main(): void {
                const server = createServer((
                    request: { method: string; url: string },
                    response: { statusCode: number; setHeader: (name: string, value: string) => boolean; write: (chunk: string) => boolean; end: (chunk: string) => boolean }
                ): boolean => true);
                server.listen(70000);
            }"#,
        )
        .unwrap();
    let unhandled = dir.join("unhandled-error");
    build_with_link_mode(
        &entry,
        &unhandled,
        &[],
        &[],
        &[],
        &dir.join("registry-unhandled"),
        &[],
        true,
    )
    .unwrap();
    let failed = Command::new(&unhandled).output().unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("Unhandled 'error' event"));
    drop(occupied_listener);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bare_import_resolution_errors_include_source_location() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-missing-bare-import-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "\n\nimport { missing } from \"not-installed\";\nfunction main(): void {}\n",
    )
    .unwrap();
    let error = build(
        &entry,
        &dir.join("app"),
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
    )
    .unwrap_err();
    assert!(error.contains("main.ts:3:"), "{error}");
    assert!(error.contains("not-installed"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_and_runs_embedded_wasi_preview1_module() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-wasi-preview1-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("wasi-fixture");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function run(): number;\n",
    )
    .unwrap();
    std::fs::write(
            package.join("bundle.js"),
            r#"module.exports = { run: function () {
                 var options = { args: ['embedded'], env: { MODE: 'standalone' }, preopens: {}, returnOnExit: true, version: 'preview1' };
                 var imports = { wasi_snapshot_preview1: Object.freeze({ __thawWasiOptions: JSON.stringify(options) }) };
                 var source = new TextEncoder().encode(`(module
                   (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
                   (memory (export "memory") 1)
                   (func (export "_start") i32.const 6 call $exit))`);
                 var instance = new WebAssembly.Instance(new WebAssembly.Module(source), imports);
                 try { instance.exports._start(); } catch (error) { if (error && error.__thawWasiExit !== undefined) return Number(error.__thawWasiExit); throw error; }
                 return 0;
               } };
"#,
        )
        .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { run } from \"wasi-fixture\"; function main(): void { console.log(run()); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "6\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn imports_a_scoped_package_subpath_with_default_and_namespace_forms() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-scoped-subpath-import-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("@scope/tools/subpaths/feature");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export default function double(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = function(value) { return value * 2; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import double from "@scope/tools/feature";
                import * as feature from "@scope/tools/feature";
                function main(): void {
                    console.log(double(20));
                    console.log(feature.double(21));
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "40\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registers_builds_and_runs_an_installed_npm_wildcard_subpath() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-installed-wildcard-{}",
        std::process::id()
    ));
    let node_modules = dir.join("node_modules");
    let package = node_modules.join("feature-kit");
    std::fs::create_dir_all(package.join("dist/features")).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{
                "name":"feature-kit",
                "version":"1.2.3",
                "types":"./index.d.ts",
                "main":"./index.js",
                "exports":{
                    ".":{"types":"./index.d.ts","require":"./index.js"},
                    "./features/*":{
                        "types":"./dist/features/*.d.ts",
                        "require":"./dist/features/*.js"
                    }
                }
            }"#,
    )
    .unwrap();
    std::fs::write(
        package.join("index.d.ts"),
        "export declare function root(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("index.js"),
        "module.exports = { root: function() { return 1; } };\n",
    )
    .unwrap();
    std::fs::write(
        package.join("dist/features/triple.d.ts"),
        "export default function triple(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("dist/features/triple.js"),
        "module.exports = function(value) { return value * 3; };\n",
    )
    .unwrap();
    let registry = dir.join("registry");
    thaw_registry::add_installed(&registry, &node_modules, "feature-kit").unwrap();

    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import triple from "feature-kit/features/triple";
                function main(): void { console.log(triple(14)); }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_package_subpath_reports_the_import_location() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-missing-subpath-{}", std::process::id()));
    let registry = dir.join("registry");
    std::fs::create_dir_all(registry.join("math-kit")).unwrap();
    std::fs::write(
        registry.join("math-kit/package.d.ts"),
        "export declare function add(a: number, b: number): number;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "\n\nimport { square } from \"math-kit/missing\";\nfunction main(): void {}\n",
    )
    .unwrap();
    let error = build(&entry, &dir.join("app"), &[], &[], &[], &registry, &[]).unwrap_err();
    assert!(error.contains("main.ts:3:"), "{error}");
    assert!(error.contains("math-kit/missing"), "{error}");
    assert!(error.contains("package.d.ts"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runs_a_multifile_lambda_with_two_bare_import_packages() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-bare-import-lambda-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    for (package, value) in [("left-mark", 20), ("right-mark", 22)] {
        std::fs::create_dir_all(registry.join(package)).unwrap();
        std::fs::write(
            registry.join(package).join("package.d.ts"),
            "export declare function mark(): number;\n",
        )
        .unwrap();
        std::fs::write(
            registry.join(package).join("bundle.js"),
            format!("module.exports = {{ mark: function() {{ return {value}; }} }};\n"),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("work.ts"),
        r#"
                import { mark as leftMark } from "left-mark";
                import { mark as rightMark } from "right-mark";
                export async function work(): Promise<void> {
                    await sleep(1);
                    console.log(leftMark() + rightMark());
                }
            "#,
    )
    .unwrap();
    let entry = dir.join("handler.ts");
    std::fs::write(
        &entry,
        r#"
                import { work } from "./work";
                async function handler(event: Json): Promise<Json> {
                    await work();
                    return event;
                }
            "#,
    )
    .unwrap();
    let output = dir.join("bootstrap");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = conn.read(&mut request).unwrap();
        let event = "{\"packages\":2}";
        conn.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: packages-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("package Lambda handler did not post a response");
    server.join().unwrap();
    let _ = child.kill();
    let result = child.wait_with_output().unwrap();
    assert!(request.starts_with("POST /2018-06-01/runtime/invocation/packages-request/response"));
    assert!(request.ends_with("{\"packages\":2}"));
    assert!(String::from_utf8_lossy(&result.stdout).contains("42"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn qualifier_identifier_passes_through_unscoped_names() {
    assert_eq!(qualifier_identifier("qs"), "qs");
    assert_eq!(qualifier_identifier("left-pad"), "left-pad");
}

#[test]
fn qualifier_identifier_uses_the_last_segment_of_a_scoped_name() {
    assert_eq!(qualifier_identifier("@hapi/hoek"), "hoek");
    assert_eq!(qualifier_identifier("@babel/core"), "core");
}

#[test]
fn qualifier_identifiers_disambiguate_equal_scoped_package_tails() {
    let qualifiers = package_qualifier_identifiers(["@foo/utils", "@bar/utils", "@hapi/hoek"]);
    assert_eq!(qualifiers["@foo/utils"], "_foo_utils");
    assert_eq!(qualifiers["@bar/utils"], "_bar_utils");
    assert_eq!(qualifiers["@hapi/hoek"], "hoek");
}

#[test]
fn qualifier_identifiers_remain_unique_after_sanitization() {
    let qualifiers =
        package_qualifier_identifiers(["@foo-bar/utils", "@foo_bar/utils", "_foo_bar_utils"]);
    let unique = qualifiers
        .values()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), qualifiers.len());
    assert!(qualifiers.values().all(|qualifier| qualifier
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')));
}

#[test]
fn sanitize_identifier_replaces_non_alphanumerics() {
    assert_eq!(sanitize_identifier("qs"), "qs");
    assert_eq!(sanitize_identifier("@hapi/hoek"), "_hapi_hoek");
    assert_eq!(sanitize_identifier("left-pad"), "left_pad");
}

#[test]
fn reads_versioned_ffi_error_abi_metadata() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-ffi-metadata-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
        &path,
        r#"{"version":1,"functions":{"externalRead":{"errorAbi":"thaw-result"}}}"#,
    )
    .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["externalRead"],
        FfiMetadata {
            error_abi: thaw_hir::FfiErrorAbi::ThawResult,
            return_ownership: thaw_hir::FfiOwnership::Borrowed,
            error_ownership: thaw_hir::FfiOwnership::Borrowed,
            param_string_abis: None,
            return_string_abi: thaw_hir::FfiStringAbi::NullTerminated,
            calling_convention: thaw_hir::FfiCallingConvention::C,
            aggregate_return_abi: thaw_hir::FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        }
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_unknown_ffi_metadata_versions_and_abis() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-invalid-ffi-metadata-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let version = dir.join("version.json");
    std::fs::write(&version, r#"{"version":5,"functions":{}}"#).unwrap();
    assert!(read_ffi_metadata(&[version])
        .unwrap_err()
        .contains("version 1"));
    let abi = dir.join("abi.json");
    std::fs::write(
        &abi,
        r#"{"version":1,"functions":{"f":{"errorAbi":"errno"}}}"#,
    )
    .unwrap();
    assert!(read_ffi_metadata(&[abi])
        .unwrap_err()
        .contains("unknown errorAbi"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_version_two_ffi_ownership_metadata() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-ffi-ownership-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
            &path,
            r#"{"version":2,"functions":{"read":{"errorAbi":"thaw-result","returnOwnership":"owned","destroy":"free_read","errorOwnership":"arena-copy","errorDestroy":"free_error"}}}"#,
        )
        .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["read"],
        FfiMetadata {
            error_abi: thaw_hir::FfiErrorAbi::ThawResult,
            return_ownership: thaw_hir::FfiOwnership::Owned {
                destroy: "free_read".into()
            },
            error_ownership: thaw_hir::FfiOwnership::ArenaCopy {
                destroy: Some("free_error".into())
            },
            param_string_abis: None,
            return_string_abi: thaw_hir::FfiStringAbi::NullTerminated,
            calling_convention: thaw_hir::FfiCallingConvention::C,
            aggregate_return_abi: thaw_hir::FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        }
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_version_three_string_abi_metadata() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-ffi-string-abi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
            &path,
            r#"{"version":3,"functions":{"slice":{"errorAbi":"direct","parameterStringAbis":["pointer-length"],"returnStringAbi":"pointer-length","callingConvention":"fast","aggregateReturnAbi":"portable"},"record":{"errorAbi":"direct","aggregateReturnAbi":"packed"}}}"#,
        )
        .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["slice"],
        FfiMetadata {
            error_abi: thaw_hir::FfiErrorAbi::Direct,
            return_ownership: thaw_hir::FfiOwnership::Borrowed,
            error_ownership: thaw_hir::FfiOwnership::Borrowed,
            param_string_abis: Some(vec![thaw_hir::FfiStringAbi::PointerLength]),
            return_string_abi: thaw_hir::FfiStringAbi::PointerLength,
            calling_convention: thaw_hir::FfiCallingConvention::Fast,
            aggregate_return_abi: thaw_hir::FfiAggregateAbi::Portable,
            aggregate_return_layout: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        }
    );
    assert_eq!(
        metadata["record"].aggregate_return_abi,
        thaw_hir::FfiAggregateAbi::Packed
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_version_four_explicit_aggregate_layout() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-ffi-aggregate-layout-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
            &path,
            r#"{"version":4,"functions":{"record":{"errorAbi":"direct","aggregateReturnAbi":"portable","aggregateReturnLayout":{"fieldOffsets":[0,16],"fieldLayouts":[null,{"fieldOffsets":[0],"size":8,"alignment":8}],"bitFields":[{"bitOffset":2,"bitWidth":5,"storageBytes":1,"signed":true},null],"size":32,"alignment":32,"indirect":true}},"register":{"errorAbi":"direct","aggregateReturnAbi":"portable","variadicAbi":"i64","aggregateReturnLayout":{"fieldOffsets":[0,8],"size":16,"alignment":8,"indirect":false,"registerClasses":["integer","sse"]}}}}"#,
        )
        .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["record"].aggregate_return_layout,
        Some(thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 16],
            field_layouts: vec![
                None,
                Some(Box::new(thaw_hir::FfiAggregateLayout {
                    field_offsets: vec![0],
                    field_layouts: vec![None],
                    field_bitfields: vec![None],
                    register_classes: vec![],
                    size: 8,
                    alignment: 8,
                    indirect: false,
                })),
            ],
            field_bitfields: vec![
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 2,
                    bit_width: 5,
                    storage_bytes: 1,
                    signed: true,
                }),
                None,
            ],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        })
    );
    assert_eq!(
        metadata["register"].variadic_abi,
        thaw_hir::FfiVariadicAbi::I64
    );
    assert_eq!(
        metadata["register"]
            .aggregate_return_layout
            .as_ref()
            .unwrap()
            .register_classes,
        vec![
            thaw_hir::FfiRegisterClass::Integer,
            thaw_hir::FfiRegisterClass::Sse,
        ]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_native_addon_builds_and_runs_end_to_end() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-native-addon-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("native-add");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function add(a: number, b: number): number;\n",
    )
    .unwrap();
    let addon_c = dir.join("addon.c");
    std::fs::write(&addon_c, r#"
            #include <stddef.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            static napi_value add(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
                napi_create_double(env, a + b, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                (void)exports;
                napi_value fn; napi_create_function(env, "add", 3, add, 0, &fn);
                return fn;
            }
        "#).unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(package.join("native.node"))
        .status()
        .unwrap()
        .success());
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "import { add } from \"native-add\"; function main(): void { console.log(add(20, 22)); }\n",
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_native_addon_class_method_builds_and_runs_end_to_end() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-native-addon-class-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("native-box");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
            package.join("package.d.ts"),
            "export declare class NativeBox { constructor(value: number); static twice(value: number): number; static get version(): number; static set version(value: number); get value(): number; set value(value: number); get(): number; add(delta?: number): number; sum(...values: number[]): number; getLater(callback: (error: Json, result: Json) => void): number; }\n",
        )
        .unwrap();
    let addon_c = dir.join("addon.c");
    std::fs::write(
            &addon_c,
            r#"
            #include <stddef.h>
            #include <stdlib.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            typedef napi_value (*napi_callback)(napi_env,napi_callback_info);
            typedef struct { const char* utf8name; napi_value name; napi_callback method; napi_callback getter; napi_callback setter; napi_value value; unsigned attributes; void* data; } napi_property_descriptor;
            typedef struct { double value; } native_box;
            static double box_version = 1;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_get_null(napi_env, napi_value*);
            extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            extern napi_status napi_wrap(napi_env, napi_value, void*, void (*)(napi_env,void*,void*), void*, void**);
            extern napi_status napi_unwrap(napi_env, napi_value, void**);
            extern napi_status napi_define_class(napi_env, const char*, size_t, napi_callback, void*, size_t, const napi_property_descriptor*, napi_value*);
            static void finalize_box(napi_env env, void* data, void* hint) { (void)env; (void)hint; free(data); }
            static napi_value box_new(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; double value;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_get_value_double(env, arg, &value);
                native_box* box = malloc(sizeof(*box)); box->value = value;
                napi_wrap(env, self, box, finalize_box, 0, 0); return self;
            }
            static napi_value box_get(napi_env env, napi_callback_info info) {
                size_t argc = 0; napi_value self, result; native_box* box;
                napi_get_cb_info(env, info, &argc, 0, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_create_double(env, box->value, &result); return result;
            }
            static napi_value box_twice(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; double value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &value);
                napi_create_double(env, value * 2, &result); return result;
            }
            static napi_value box_set(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; native_box* box; double value;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_get_value_double(env, arg, &value); box->value = value; return arg;
            }
            static napi_value box_get_version(napi_env env, napi_callback_info info) {
                napi_value result; (void)info;
                napi_create_double(env, box_version, &result); return result;
            }
            static napi_value box_set_version(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg; double value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &value); box_version = value; return arg;
            }
            static napi_value box_add(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self, result; native_box* box; double delta = 0;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                if (argc == 1) napi_get_value_double(env, arg, &delta);
                napi_create_double(env, box->value + delta, &result); return result;
            }
            static napi_value box_sum(napi_env env, napi_callback_info info) {
                size_t argc = 8; napi_value args[8], self, result; double sum = 0, value;
                napi_get_cb_info(env, info, &argc, args, &self, 0);
                for (size_t index = 0; index < argc; index++) {
                    napi_get_value_double(env, args[index], &value); sum += value;
                }
                napi_create_double(env, sum, &result); return result;
            }
            static napi_value box_get_later(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value callback, self, callback_args[2], ignored, queued; native_box* box;
                napi_get_cb_info(env, info, &argc, &callback, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_get_null(env, &callback_args[0]);
                napi_create_double(env, box->value, &callback_args[1]);
                napi_call_function(env, self, callback, 2, callback_args, &ignored);
                napi_create_double(env, 1, &queued); return queued;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value constructor;
                napi_property_descriptor properties[7] = {
                    { "get", 0, box_get, 0, 0, 0, 0, 0 },
                    { "value", 0, 0, box_get, box_set, 0, 0, 0 },
                    { "add", 0, box_add, 0, 0, 0, 0, 0 },
                    { "sum", 0, box_sum, 0, 0, 0, 0, 0 },
                    { "getLater", 0, box_get_later, 0, 0, 0, 0, 0 },
                    { "twice", 0, box_twice, 0, 0, 0, 1024, 0 },
                    { "version", 0, 0, box_get_version, box_set_version, 0, 1024, 0 }
                };
                napi_define_class(env, "NativeBox", 9, box_new, 0, 7, properties, &constructor);
                napi_set_named_property(env, exports, "NativeBox", constructor);
                return exports;
            }
        "#,
        )
        .unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(package.join("native.node"))
        .status()
        .unwrap()
        .success());
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            "import { NativeBox } from \"native-box\"; function main(): void { console.log(NativeBox.twice(21)); console.log(NativeBox.version); console.log(NativeBox.version = 3); console.log(NativeBox.version); const box: JsValue = new NativeBox(42); console.log(box.value); console.log(box.value = 10); console.log(box.value); console.log(box.get()); console.log(box.add()); console.log(box.add(8)); console.log(box.sum()); console.log(box.sum(1, 2, 3)); const callback = (error: Json, result: Json): void => { console.log(Number(result)); }; console.log(box.getLater(callback)); }\n",
        )
        .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "42\n1\n3\n3\n42\n10\n10\n10\n10\n18\n0\n6\n10\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_runs_utf8_validate_prebuild_when_supplied() {
    let Ok(prebuild) = std::env::var("THAW_UTF8_VALIDATE_NODE") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("thaw-cli-utf8-validate-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("utf-8-validate");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function isValidUTF8(buffer: any): boolean;\n",
    )
    .unwrap();
    std::fs::copy(prebuild, package.join("native.node")).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[240,144,128,128]}]"))));
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[255]}]"))));
            }
            "#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["utf-8-validate".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_runs_bcrypt_async_callbacks_when_supplied() {
    let Ok(prebuild) = std::env::var("THAW_BCRYPT_NODE") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("thaw-cli-bcrypt-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("bcrypt");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function gen_salt(argsArray: Json): Json;\n\
             declare function encrypt(argsArray: Json): Json;\n\
             declare function compare(argsArray: Json): Json;\n",
    )
    .unwrap();
    std::fs::copy(prebuild, package.join("native.node")).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["bcrypt".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.lines().any(|line| line.starts_with("$2b$04$")),
        "{stdout}"
    );
    assert!(stdout.lines().any(|line| line == "false"), "{stdout}");
    assert!(
        stdout.lines().any(|line| line.contains("Invalid salt")),
        "{stdout}"
    );
    assert_eq!(stdout.lines().count(), 4, "{stdout}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_bcrypt_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-bcrypt-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "bcrypt@6.0.0").unwrap();
    let native = added.native_addon.expect("a matching prebuild is bundled");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(registry.join("bcrypt/native.node").is_file());
    assert!(registry.join("bcrypt/native-addon.json").is_file());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                const salt: Json = callNativeAddon(
                    "gen_salt_sync",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]")
                );
                console.log(String(salt));
                const hash: Json = callNativeAddon(
                    "encrypt_sync",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                );
                console.log(String(hash));
                console.log(Boolean(callNativeAddon(
                    "compare_sync",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                )));
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["bcrypt".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 7, "{stdout}");
    assert!(lines[0].starts_with("$2b$04$"), "{stdout}");
    assert!(lines[1].starts_with("$2b$04$"), "{stdout}");
    assert_eq!(lines[2], "false", "{stdout}");
    assert!(
        lines[3..].iter().any(|line| line.starts_with("$2b$04$")),
        "{stdout}"
    );
    assert!(lines[3..].contains(&"false"), "{stdout}");
    assert!(
        lines[3..].iter().any(|line| line.contains("Invalid salt")),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_loads_sqlite3_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-sqlite3-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "sqlite3@5.1.7").unwrap();
    let native = added
        .native_addon
        .expect("the official GitHub prebuild should be downloaded");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(native.source.contains("TryGhost/node-sqlite3/releases"));

    // Keep this fixture focused on construction and one error-first
    // callback method rather than sqlite3's full overloaded surface.
    std::fs::write(
            registry.join("sqlite3/package.d.ts"),
            "export declare class Database { constructor(filename: string); close(callback: (error: Error | null) => void): void; }\n",
        )
        .unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "import { Database } from \"sqlite3\";\n\
             function main(): void {\n\
                 const database: JsValue = new Database(\":memory:\");\n\
                 const onClose = (error: Json): void => { console.log(\"sqlite3-closed\"); };\n\
                 database.close(onClose);\n\
                 console.log(\"sqlite3-constructed\");\n\
             }\n",
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["sqlite3".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "sqlite3-constructed\nsqlite3-closed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_utf8_validate_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-utf8-validate-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "utf-8-validate@6.0.6").unwrap();
    let native = added.native_addon.expect("a matching prebuild is bundled");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(registry.join("utf-8-validate/native.node").is_file());
    assert!(registry.join("utf-8-validate/native-addon.json").is_file());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[240,144,128,128]}]"))));
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[255]}]"))));
            }
            "#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["utf-8-validate".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_real_p_limit_promise_workload_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-p-limit-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "p-limit@2.3.0").unwrap();
    assert_eq!(
        added.dependency_versions.get("p-try").map(String::as_str),
        Some("2.2.0")
    );
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import pLimit from "p-limit";
                function main(): void {
                    const limit: JsValue = pLimit(2);
                    const task: JsValue = getDynamicValue("Number");
                    const result: Json = callDynamicValueWithValue(limit, task);
                    console.log(Number(result));
                }"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_runs_a_real_esm_package_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-has-flag-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "has-flag@5.0.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import hasFlag from "has-flag";
                function main(): void {
                    const present: Json = hasFlag(JSON.parse("[\"--thaw-parser-backed-bundler\"]"));
                    console.log(Boolean(present));
                }"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "false\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_parcel_watcher_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-parcel-watcher-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let watched = dir.join("watched");
    let snapshot = dir.join("snapshot.bin");
    std::fs::create_dir_all(&watched).unwrap();
    std::fs::write(watched.join("before.txt"), "before").unwrap();
    let added = thaw_registry::add(&registry, "@parcel/watcher@2.5.1").unwrap();
    let native = added
        .native_addon
        .expect("the platform optional dependency contains a prebuild");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(native
        .source
        .contains("watcher-linux-x64-glibc/watcher.node"));

    let args = serde_json::to_string(&serde_json::json!([
        watched.to_string_lossy(),
        snapshot.to_string_lossy(),
        {}
    ]))
    .unwrap();
    let args_literal = serde_json::to_string(&args).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            format!(
                "function main(): void {{ const result: Json = writeSnapshot(JSON.parse({args_literal})); console.log(\"snapshot-created\"); }}\n"
            ),
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["@parcel/watcher".into()],
    )
    .unwrap();
    let event_file = watched.join("event.txt");
    let subscribe_args =
        serde_json::to_string(&serde_json::json!([watched.to_string_lossy()])).unwrap();
    let subscribe_literal = serde_json::to_string(&subscribe_args).unwrap();
    let event_source = dir.join("events.ts");
    let event_output = dir.join("events-app");
    std::fs::write(
            &event_source,
            format!(
                r#"import * as fs from "node:fs";
                function main(): void {{
                    let received: number = 0;
                    const callback = (error: Json, result: Json): Json => {{
                        received = 1;
                        return result;
                    }};
                    const subscribed: Json = callNativeAddonWithCallback("subscribe", JSON.parse({subscribe_literal}), callback);
                    fs.writeFileSync("{}", "event");
                    while (received < 1) {{
                        const count: number = pollNativeAddonEvents();
                    }}
                    const unsubscribed: Json = callNativeAddonWithCallback("unsubscribe", JSON.parse({subscribe_literal}), callback);
                    console.log("watch-event");
                }}
                "#,
                event_file.to_string_lossy()
            ),
        )
        .unwrap();
    build(
        &event_source,
        &event_output,
        &[],
        &[],
        &[],
        &registry,
        &["@parcel/watcher".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "snapshot-created\n"
    );
    assert!(snapshot.is_file());
    let result = Command::new(&event_output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "watch-event\n");
    assert!(event_file.is_file());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rewrite_qualified_calls_is_a_no_op_with_no_rewrites() {
    let source = "function main(): void { console.log(qs.stringify(x)); }";
    assert_eq!(rewrite_qualified_calls(source, &[]).unwrap(), source);
}

#[test]
fn rewrite_qualified_calls_replaces_matching_qualified_calls_only() {
    let source = "function main(): void {\n\
             console.log(qs.stringify(x));\n\
             console.log(hoek.stringify(y));\n\
             console.log(qs.parse(z));\n\
             console.log(unrelated.stringify(w));\n\
         }";
    let rewrites = vec![
        (
            "qs".to_string(),
            "stringify".to_string(),
            "qs_stringify".to_string(),
        ),
        (
            "hoek".to_string(),
            "stringify".to_string(),
            "hoek_stringify".to_string(),
        ),
    ];
    let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();

    assert!(rewritten.contains("console.log(qs_stringify(x));"));
    assert!(rewritten.contains("console.log(hoek_stringify(y));"));
    // Not in `rewrites` (no collision for `parse`, or the object
    // isn't a known qualifier at all) -- left completely alone.
    assert!(rewritten.contains("console.log(qs.parse(z));"));
    assert!(rewritten.contains("console.log(unrelated.stringify(w));"));
}

#[test]
fn rewrite_qualified_calls_handles_a_call_nested_in_an_expression() {
    let source = "function main(): void { const r = String(qs.stringify(x)); }";
    let rewrites = vec![(
        "qs".to_string(),
        "stringify".to_string(),
        "qs_stringify".to_string(),
    )];
    let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();
    assert!(rewritten.contains("const r = String(qs_stringify(x));"));
}

#[test]
fn rewrites_external_class_constructors_without_touching_other_new_expressions() {
    let source = "const a = new Database(\":memory:\"); const b = new sqlite3.Database(\"db.sqlite\"); const c = new LocalBox(1);";
    let rewritten =
        rewrite_external_class_constructors(source, &[("sqlite3".into(), "Database".into())])
            .unwrap();
    assert_eq!(
            rewritten,
            "const a = Database(\":memory:\"); const b = sqlite3.Database(\"db.sqlite\"); const c = new LocalBox(1);"
        );
}

#[test]
fn rewrites_methods_on_values_created_from_external_classes() {
    let source = "const db = new Database(\":memory:\"); db.configure(\"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);";
    let rewritten = rewrite_external_class_methods(
        source,
        &[("sqlite3".into(), "Database".into())],
        &[(
            "Database".into(),
            "configure".into(),
            "__thaw_configure".into(),
            2,
            false,
            vec![thaw_hir::HirType::Str, thaw_hir::HirType::F64],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const db = new Database(\":memory:\"); __thaw_configure(db, \"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);"
        );
}

#[test]
fn rewrites_named_and_namespace_static_class_methods() {
    let source = "NativeBox.create(1); addon.NativeBox.create(\"text\"); LocalBox.create(2);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[],
        &[],
        &[
            (
                "addon".into(),
                "NativeBox".into(),
                "create".into(),
                "__thaw_create_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "addon".into(),
                "NativeBox".into(),
                "create".into(),
                "__thaw_create_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "__thaw_create_number(1); __thaw_create_string(\"text\"); LocalBox.create(2);"
    );
}

#[test]
fn rewrites_typed_napi_instance_getters() {
    let source = "const box = new NativeBox(42); console.log(box.value);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[],
        &[],
        &[(
            "NativeBox".into(),
            "value".into(),
            "__thaw_get_value".into(),
        )],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = new NativeBox(42); console.log(__thaw_get_value(box));"
    );
}

#[test]
fn rewrites_typed_napi_instance_setters_and_preserves_expression_values() {
    let source = "const box = new NativeBox(42); const assigned: number = box.value = 7;";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[],
        &[],
        &[],
        &[(
            "NativeBox".into(),
            "value".into(),
            "__thaw_set_value".into(),
            thaw_hir::HirType::F64,
        )],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = new NativeBox(42); const assigned: number = __thaw_set_value(box, 7);"
    );
}

#[test]
fn rewrites_named_and_namespace_static_accessors() {
    let source = "console.log(NativeBox.version); addon.NativeBox.version = 7; console.log(addon.NativeBox.version);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[(
            "addon".into(),
            "NativeBox".into(),
            "version".into(),
            "__thaw_get_version".into(),
        )],
        &[(
            "addon".into(),
            "NativeBox".into(),
            "version".into(),
            "__thaw_set_version".into(),
            thaw_hir::HirType::F64,
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "console.log(__thaw_get_version()); __thaw_set_version(7); console.log(__thaw_get_version());"
        );
}

#[test]
fn generates_typed_napi_static_method_shims_without_instance_receivers() {
    let class = thaw_bridge::DtsClass {
        name: "NativeBox".into(),
        extends: None,
        constructors: vec![],
        methods: vec![thaw_bridge::DtsMethod {
            name: "create".into(),
            params: vec![(
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            )],
            required_params: 1,
            rest_param: None,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            is_static: true,
            kind: thaw_bridge::DtsMethodKind::Method,
            overloaded: false,
        }],
        properties: vec![],
    };
    let mut shim = String::new();
    let generated = generate_napi_class_method_overloads(
        &class,
        true,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].0, "create");
    assert!(shim.contains("(value: number): number;"));
    assert!(!shim.contains("receiver"));
}

#[test]
fn rewrites_zero_argument_external_class_methods() {
    let source = "const box = new NativeBox(42); const value = box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = new NativeBox(42); const value = __thaw_get(box);"
    );
}

#[test]
fn tracks_external_class_instance_aliases_and_invalidates_reassignments() {
    let source = "const box = new NativeBox(42); const alias = box; alias.get(); let assigned = alias; assigned.get(); assigned = box; assigned.get(); assigned = unknown; assigned.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = new NativeBox(42); const alias = box; __thaw_get(alias); let assigned = alias; __thaw_get(assigned); assigned = box; __thaw_get(assigned); assigned = unknown; assigned.get();"
        );
}

#[test]
fn tracks_external_class_instances_through_object_properties() {
    let source = "const box = new NativeBox(42); const holder = { box }; holder.box.get(); holder[\"box\"].get(); const nested = { inner: { value: new NativeBox(7) } }; nested.inner.value.get(); holder.box = box; holder.box.get(); holder.box = unknown; holder.box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = new NativeBox(42); const holder = { box }; __thaw_get(holder.box); __thaw_get(holder[\"box\"]); const nested = { inner: { value: new NativeBox(7) } }; __thaw_get(nested.inner.value); holder.box = box; __thaw_get(holder.box); holder.box = unknown; holder.box.get();"
        );
}

#[test]
fn joins_object_property_instance_facts_across_branches() {
    let source = "const box = new NativeBox(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } holder.box.get(); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = new NativeBox(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } __thaw_get(holder.box); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();"
        );
}

#[test]
fn selects_external_method_overloads_by_arity_and_callback_shape() {
    let source = "const db = new Database(\":memory:\"); const done = (error: Json): void => {}; db.run(\"select 1\"); db.run(\"select 1\", done);";
    let rewritten = rewrite_external_class_methods(
        source,
        &[("sqlite3".into(), "Database".into())],
        &[
            (
                "Database".into(),
                "run".into(),
                "__run_sync".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "Database".into(),
                "run".into(),
                "__run_callback".into(),
                2,
                true,
                vec![
                    thaw_hir::HirType::Str,
                    thaw_hir::HirType::Function(
                        vec![thaw_hir::HirType::Json],
                        Box::new(thaw_hir::HirType::Void),
                    ),
                ],
            ),
        ],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const db = new Database(\":memory:\"); const done = (error: Json): void => {}; __run_sync(db, \"select 1\"); __run_callback(db, \"select 1\", done);"
        );
}

#[test]
fn selects_same_arity_external_method_overloads_by_argument_type() {
    let source = "const box = new NativeBox(1); const n = 42; const s = \"hello\"; box.set(n); box.set(s); box.set(7); box.set(\"world\");";
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = new NativeBox(1); const n = 42; const s = \"hello\"; __set_number(box, n); __set_string(box, s); __set_number(box, 7); __set_string(box, \"world\");"
        );
}

#[test]
fn infers_external_overload_types_from_composed_expressions() {
    let source = r#"const box = new NativeBox(1); const n = 20 + 22; const s = "hel" + "lo"; const b = n > 0; const config = { n, nested: { text: s }, enabled: b }; box.set(n); box.set(s); box.set(b); box.set(Number("7")); box.set(`value-${s}`); box.set(true ? "yes" : "no"); box.set(config.n); box.set(config.nested.text); box.set(config.enabled); box.set(({ value: 7 }).value);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, n)"));
    assert!(rewritten.contains("__set_string(box, s)"));
    assert!(rewritten.contains("__set_boolean(box, b)"));
    assert!(rewritten.contains("__set_number(box, Number(\"7\"))"));
    assert!(rewritten.contains("__set_string(box, `value-${s}`)"));
    assert!(rewritten.contains("__set_string(box, true ? \"yes\" : \"no\")"));
    assert!(rewritten.contains("__set_number(box, config.n)"));
    assert!(rewritten.contains("__set_string(box, config.nested.text)"));
    assert!(rewritten.contains("__set_boolean(box, config.enabled)"));
    assert!(rewritten.contains("__set_number(box, ({ value: 7 }).value)"));
}

#[test]
fn infers_external_overload_types_from_user_function_returns_and_forward_references() {
    let source = r#"const box = new NativeBox(1); const makeText = (): string => "text"; const makeFlag = function(): boolean { return true; }; const inferredFlag = () => true; box.set(makeNumber()); box.set(makeText()); box.set(makeFlag()); box.set(inferredNumber()); box.set(inferredText()); box.set(inferredFlag()); function makeNumber(): number { return 42; } function inferredNumber() { return 40 + 2; } function inferredText() { return forwardText(); } function forwardText() { return "text"; }"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, makeNumber())"));
    assert!(rewritten.contains("__set_string(box, makeText())"));
    assert!(rewritten.contains("__set_boolean(box, makeFlag())"));
    assert!(rewritten.contains("__set_number(box, inferredNumber())"));
    assert!(rewritten.contains("__set_string(box, inferredText())"));
    assert!(rewritten.contains("__set_boolean(box, inferredFlag())"));
}

#[test]
fn selects_object_overloads_from_structural_property_types() {
    let source = r#"function makeNumeric(): { value: number } { return { value: 11 }; } const box = new NativeBox(1); const numeric = { value: 42 }; const textual = { value: "text" }; const choose = true; box.configure(numeric); box.configure(textual); box.configure({ value: 7 }); box.configure({ ["value"]: "computed" }); box.configure({ ...numeric }); box.configure({ ...numeric, value: "override" }); box.configure({ ...{ value: 9 } }); box.configure({ ...makeNumeric() }); box.configure({ ...(choose ? makeNumeric() : numeric) });"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::F64,
                )])],
            ),
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::Str,
                )])],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__configure_number(box, numeric)"));
    assert!(rewritten.contains("__configure_string(box, textual)"));
    assert!(rewritten.contains("__configure_number(box, { value: 7 })"));
    assert!(rewritten.contains("__configure_string(box, { [\"value\"]: \"computed\" })"));
    assert!(rewritten.contains("__configure_number(box, { ...numeric })"));
    assert!(rewritten.contains("__configure_string(box, { ...numeric, value: \"override\" })"));
    assert!(rewritten.contains("__configure_number(box, { ...{ value: 9 } })"));
    assert!(rewritten.contains("__configure_number(box, { ...makeNumeric() })"));
    assert!(
        rewritten.contains("__configure_number(box, { ...(choose ? makeNumeric() : numeric) })")
    );
}

#[test]
fn tracks_assignment_flow_for_variables_and_nested_object_properties() {
    let source = r#"const box = new NativeBox(1); let value = 42; box.set(value); value = "text"; box.set(value); const config = { nested: { value: 1 }, direct: true }; config.nested.value = "nested"; config["direct"] = 7; box.set(config.nested.value); box.set(config.direct);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, value); value = \"text\""));
    assert!(rewritten.contains("__set_string(box, value); const config"));
    assert!(rewritten.contains("__set_string(box, config.nested.value)"));
    assert!(rewritten.contains("__set_number(box, config.direct)"));
}

#[test]
fn joins_if_branch_types_and_discards_conflicting_facts() {
    let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; if (flag) { stable = 2; } else { stable = 3; } box.set(stable); let conflict = 1; if (flag) { conflict = "text"; box.set(conflict); } else { conflict = 2; box.set(conflict); } box.set(conflict); let oneSided = 1; if (flag) { oneSided = "changed"; } box.set(oneSided);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("conflict = \"text\"; __set_string(box, conflict)"));
    assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, oneSided)"));
}

#[test]
fn joins_switch_fallthrough_break_and_no_match_paths() {
    let source = r#"const choice = 1; const flag = true; const box = new NativeBox(1); let stable = 1; switch (choice) { case 0: stable = 2; break; default: stable = 3; } box.set(stable); let conflict = 1; switch (choice) { case 0: conflict = "text"; break; default: conflict = 2; } box.set(conflict); let noDefault = 1; switch (choice) { case 0: noDefault = "text"; break; } box.set(noDefault); let fallen = 1; switch (choice) { case 0: fallen = "temporary"; case 1: fallen = 2; break; default: fallen = 3; } box.set(fallen); let guarded = 1; switch (choice) { case 0: if (flag) { guarded = "text"; break; guarded = 4; } guarded = 2; break; default: guarded = 3; } box.set(guarded); let loopBreak = 1; switch (choice) { case 0: while (flag) { break; } loopBreak = 2; break; default: loopBreak = 3; } box.set(loopBreak); let stableCallback = (): void => {}; switch (choice) { case 0: stableCallback = (): void => {}; break; default: stableCallback = (): void => {}; } box.use(stableCallback); let conflictCallback = (): void => {}; switch (choice) { case 0: conflictCallback = 1; break; default: conflictCallback = (): void => {}; } box.use(conflictCallback); let stableBox = new NativeBox(1); switch (choice) { case 0: stableBox = new NativeBox(2); break; default: stableBox = new NativeBox(3); } stableBox.get(); let conflictBox = new NativeBox(1); switch (choice) { case 0: conflictBox = "text"; break; default: conflictBox = new NativeBox(3); } conflictBox.get();"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "use".into(),
                "__use_value".into(),
                1,
                false,
                vec![thaw_hir::HirType::Json],
            ),
            (
                "NativeBox".into(),
                "use".into(),
                "__use_callback".into(),
                1,
                true,
                vec![thaw_hir::HirType::Json],
            ),
            (
                "NativeBox".into(),
                "get".into(),
                "__get".into(),
                0,
                false,
                Vec::new(),
            ),
        ],
    )
    .unwrap();

    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("__set_unknown(box, conflict)"));
    assert!(rewritten.contains("__set_unknown(box, noDefault)"));
    assert!(rewritten.contains("__set_number(box, fallen)"));
    assert!(rewritten.contains("__set_unknown(box, guarded)"));
    assert!(rewritten.contains("__set_number(box, loopBreak)"));
    assert!(rewritten.contains("__use_callback(box, stableCallback)"));
    assert!(rewritten.contains("__use_value(box, conflictCallback)"));
    assert!(rewritten.contains("__get(stableBox)"));
    assert!(rewritten.contains("conflictBox.get()"));
}

#[test]
fn joins_while_and_for_types_against_the_zero_iteration_path() {
    let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; while (flag) { stable = 2; box.set(stable); break; } box.set(stable); let changed = 1; while (flag) { changed = "text"; box.set(changed); break; } box.set(changed); let loopValue = 1; for (let index = 0; index < 1; index = index + 1) { box.set(index); loopValue = "loop"; box.set(loopValue); } box.set(loopValue);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("stable = 2; __set_number(box, stable)"));
    assert!(rewritten.contains("} __set_number(box, stable)"));
    assert!(rewritten.contains("changed = \"text\"; __set_string(box, changed)"));
    assert!(rewritten.contains("} __set_unknown(box, changed)"));
    assert!(rewritten.contains("__set_number(box, index)"));
    assert!(rewritten.contains("loopValue = \"loop\"; __set_string(box, loopValue)"));
    assert!(rewritten.contains("} __set_unknown(box, loopValue)"));
}

#[test]
fn joins_try_catch_paths_and_applies_finally_to_every_exit() {
    let source = r#"const box = new NativeBox(1); let stable = 1; try { stable = 2; } catch (error) { stable = 3; } box.set(stable); let conflict = 1; try { conflict = "try"; box.set(conflict); } catch (error) { conflict = 2; box.set(conflict); } box.set(conflict); let catchInput = 1; try { catchInput = "changed"; throw "fail"; } catch (error) { box.set(catchInput); } let finalized = 1; try { finalized = "try"; } catch (error) { finalized = true; } finally { finalized = 7; box.set(finalized); } box.set(finalized);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into())],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("conflict = \"try\"; __set_string(box, conflict)"));
    assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, conflict)"));
    assert!(rewritten.contains("catch (error) { __set_unknown(box, catchInput)"));
    assert!(rewritten.contains("finalized = 7; __set_number(box, finalized)"));
    assert!(rewritten.ends_with("__set_number(box, finalized);"));
}
