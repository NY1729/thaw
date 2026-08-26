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

