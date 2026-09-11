#[test]
fn registry_add_fetches_and_loads_sqlite3_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-sqlite3-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "sqlite3@6.0.1").unwrap();
    let native = added
        .native_addon
        .expect("the official GitHub prebuild should be downloaded");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(native.source.contains("TryGhost/node-sqlite3/releases"));

    // Keep the declaration fixture focused on the three methods exercised
    // here; sqlite3's full overload surface is covered by declaration tests.
    std::fs::write(
            registry.join("sqlite3/package.d.ts"),
            "export declare class Database { constructor(filename: string); exec(sql: string): void; run(sql: string, params: Json[], callback: (error: Error | null) => void): void; all(sql: string, callback: (error: Error | null, rows: any[]) => void): void; close(callback: (error: Error | null) => void): void; }\n",
        )
        .unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        "import { Database } from \"sqlite3\";\n\
             function main(): void {\n\
                 const database: JsValue = new Database(\":memory:\");\n\
                 database.exec(\"CREATE TABLE values_table (value INTEGER)\");\n\
                 database.run(\"INSERT INTO values_table(value) VALUES (?)\", [42], (error: Json): void => {\n\
                     database.all(\"SELECT value FROM values_table\", (error: Json, rows: Json): void => {\n\
                         console.log(JSON.stringify(rows));\n\
                         database.close((): void => { console.log(\"sqlite3-closed\"); });\n\
                     });\n\
                 });\n\
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
        "sqlite3-constructed\n[{\"value\":42}]\nsqlite3-closed\n"
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

/// `p-queue`'s whole export is `export default class PQueue extends
/// EventEmitter<EventName> { ... }`. Constructing a `Fallback`
/// (QuickJS) class resolves its constructor as a JS *global* named
/// after the class (`thaw_js_get_global`) -- populated, for a *named*
/// export, by the generic `module.exports` -> `globalThis` copy loop.
/// A default-exported class's `module.exports` instead *is* the class
/// itself, so that loop enumerates the class's own static properties,
/// never a property literally named after the class -- `new PQueue()`
/// used to fail at run time with `JavaScript value handle N is not a
/// constructor`. `ModuleBundle::class_names` now binds every package
/// class's name the same way a default-exported *function* already
/// was.
#[test]
fn registry_add_runs_a_real_p_queue_promise_workload_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-p-queue-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "p-queue@8.1.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import PQueue from "p-queue";
async function main(): Promise<void> {
    const queue = new PQueue({ concurrency: 2 });
    const results = await Promise.all([
        queue.add(async () => 1),
        queue.add(async () => 2),
        queue.add(async () => 3),
    ]);
    console.log(results[0] + "," + results[1] + "," + results[2]);
    console.log(queue.size + " " + queue.pending);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["p-queue".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "1,2,3\n0 0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// pino's logger is `Object.setPrototypeOf(prototype, EventEmitter.
/// prototype)` (its own `lib/proto.js`) -- a mixin that never calls
/// `EventEmitter.call(this)`, so `this._events` is never initialized by
/// the constructor. Setting the logger's level (`pino()` does this
/// during construction) calls `this.emit('level-change', ...)` before
/// any `.on()` could have lazily created `_events` -- every
/// `EventEmitter.prototype` method except `.on()`/`.once()`/etc. used to
/// assume `_events` already existed, so this crashed outright:
/// `cannot read property 'level-change' of undefined`. Fixed generally
/// in thaw-registry's `node:events` builtin (every method now tolerates
/// a missing `_events`, matching real Node); this test drives the real
/// package end to end -- construction, top-level logging, a child
/// logger, and structured data -- not just the isolated mixin shape.
#[test]
fn registry_add_logs_with_real_pino_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-pino-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "pino@9.5.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import pino from "pino";
function main(): void {
    const logger = pino();
    logger.info("hello");
    const child = logger.child({ module: "test" });
    child.warn("careful");
    logger.info({ a: 1 }, "with data");
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["pino".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Each line is a real pino JSON log record; time/hostname vary, so
    // check the stable fields rather than an exact match.
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    assert!(lines.next().unwrap().contains(r#""level":30"#));
    assert!(lines.next().unwrap().contains(r#""level":40"#));
    let with_data = lines.next().unwrap();
    assert!(with_data.contains(r#""a":1"#));
    assert!(with_data.contains(r#""msg":"with data""#));
    assert_eq!(lines.next(), None);
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
fn registry_add_builds_and_runs_yaml_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-yaml-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "yaml@2.8.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { parse, stringify } from "yaml";
                function main(): void {
                    const value: Json = parse("name: thaw\nitems:\n  - 20\n  - 22\n");
                    console.log(String(value.name));
                    console.log(Number(value.items[0]) + Number(value.items[1]));
                    const output: string = stringify({ enabled: true });
                    console.log(output);
                }"#,
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
        "thaw\n42\nenabled: true\n\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_runs_date_fns_root_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-date-fns-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "date-fns@4.1.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { weeksToDays } from "date-fns";
                function main(): void {
                    console.log(weeksToDays(6));
                }"#,
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
fn registry_add_builds_and_runs_nanoid_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-nanoid-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "nanoid@5.1.5").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { customAlphabet, nanoid } from "nanoid";
                function main(): void {
                    const first: string = nanoid();
                    const second: string = nanoid(12);
                    const makeId = customAlphabet("ab", 8);
                    const third: string = makeId();
                    const fourth: string = makeId(5);
                    const defaultMaker = customAlphabet("ab");
                    const fifth: string = defaultMaker();
                    console.log(first.length);
                    console.log(second.length);
                    console.log(first === second);
                    console.log(third.length);
                    console.log(fourth.length);
                    console.log(fifth.length);
                }"#,
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
        "21\n12\nfalse\n8\n5\n21\n"
    );
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

    let watched_literal = serde_json::to_string(&watched.to_string_lossy()).unwrap();
    let snapshot_literal = serde_json::to_string(&snapshot.to_string_lossy()).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            format!(
                "async function main(): Promise<void> {{ const result: string = await writeSnapshot({watched_literal}, {snapshot_literal}, {{ backend: \"inotify\" }}); console.log(\"snapshot-created\"); }}\n"
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
    let callback_file = watched.join("callback.txt");
    let callback_literal = serde_json::to_string(&callback_file.to_string_lossy()).unwrap();
    let event_source = dir.join("events.ts");
    let event_output = dir.join("events-app");
    std::fs::write(
            &event_source,
            format!(
                r#"import * as fs from "node:fs";
                async function main(): Promise<void> {{
                    let received: number = 0;
                    const callback = (error: Json, result: Json): Json => {{
                        received = 1;
                        fs.writeFileSync({callback_literal}, "received");
                        return result;
                    }};
                    await subscribe({watched_literal}, callback, {{ backend: "inotify" }});
                    fs.writeFileSync("{}", "event");
                    let attempts: number = 0;
                    while (received < 1 && attempts < 10000) {{
                        const count: number = pollNativeAddonEvents();
                        attempts = attempts + 1;
                    }}
                    if (received < 1) {{
                        throw new Error("watch event timed out");
                    }}
                    await unsubscribe({watched_literal}, callback, {{ backend: "inotify" }});
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
    let mut child = Command::new(&event_output).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !callback_file.is_file() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let exit_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < exit_deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let exited = child.try_wait().unwrap().is_some();
    if !exited {
        child.kill().unwrap();
    }
    child.wait().unwrap();
    assert!(event_file.is_file());
    assert!(callback_file.is_file());
    assert!(exited, "watcher process did not exit after unsubscribe");
    let _ = std::fs::remove_dir_all(dir);
}

