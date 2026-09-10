/// Real, fetched drizzle-orm -- the next breadth-first bug-hunting target
/// after uuid. drizzle-orm uses npm subpath exports extensively (the root
/// package exposes only SQL-operator helpers; the actual `sqliteTable`/
/// `pgTable` factories and column builders live under dialect-specific
/// subpaths like `drizzle-orm/sqlite-core`), which thaw-registry already
/// supports: `thaw registry add drizzle-orm` alone processes every subpath
/// at install time, and `--use drizzle-orm/sqlite-core` at build time
/// resolves against the already-installed `subpaths/sqlite-core/`
/// directory (`thaw registry add drizzle-orm/sqlite-core` directly does
/// *not* work -- it's treated as a separate package spec and fails trying
/// to `npm install` it).
///
/// `sqliteTable` (and `pgTable`, `mysqlTable`, ...) -- the central
/// primitive essentially all real drizzle schema-definition code depends
/// on -- was completely absent from the flattened `package.d.ts`, unlike
/// every previously-supported npm export shape. Root cause: drizzle-orm
/// declares it `export declare const sqliteTable: SQLiteTableFn;`
/// (`SQLiteTableFn` an interface with one call signature per overload) --
/// a `Decl::Var` binding, not `declare function`. Thaw's whole export
/// pipeline (thaw-registry's re-export flattening, thaw-bridge's `.d.ts`
/// parser) only ever recognized a function or class/interface export;
/// a plain const was silently dropped, same underlying gap as uuid's
/// `NIL`/`MAX` (see the doc comment above) but blocking rather than
/// cosmetic, since without it no real drizzle schema can be written at
/// all.
///
/// Fixed by teaching thaw-registry's flattening (`callable_const_
/// declarations` in `install.rs`) to recognize a `declare const NAME:
/// TypeRef;` whose `TypeRef` is a same-file interface with a call
/// signature, carrying both the const's and the interface's own
/// declaration text into the flattened output, and teaching thaw-bridge's
/// `parse_dts` (`extract_const_call_signature_decls`/
/// `lower_dts_call_signature`) to synthesize a `DtsFunction` per overload
/// from such an interface's call signatures -- exactly like an ordinary
/// ambient function declaration. No thaw-hir or thaw-cli shim-generation
/// changes were needed: once `sqliteTable` is just another entry in
/// `pkg.functions`, it flows through the entirely existing
/// `Classification::Fallback` call path (every overload here classifies
/// Fallback, since every parameter/return type involves unresolvable
/// generics/callbacks), identical to any other already-working npm
/// function.
///
/// Scope: this only covers making `sqliteTable`/`integer`/`text` etc.
/// *callable*. Using the *returned* table's columns
/// (`usersTable.id`) or passing it into a query builder
/// (`db.select().from(usersTable)`) is a separate, later concern, not
/// attempted here.
#[test]
fn registry_add_builds_a_sqlite_schema_with_real_drizzle_orm_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-drizzle-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "drizzle-orm").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { sqliteTable, integer, text } from "drizzle-orm/sqlite-core";
function main(): void {
    const users = sqliteTable("users", {
        id: integer(),
        name: text(),
    });
    console.log("ok");
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
        &["drizzle-orm/sqlite-core".to_string()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real drizzle-orm's query-builder chain (`db.select().from(t).where(cond)`,
/// via the `sqlite-proxy` driver) -- the scenario explicitly scoped out of
/// the test above. Every `.select()`/`.from()`/`.where()` link is a
/// "thenable" (drizzle's `QueryPromise` base class implements `.then` by
/// calling `execute()`), and `resolve_promise_value`
/// (`crates/thaw-quickjs/src/quickjs/api.rs`) used to call
/// `Promise.resolve()` unconditionally on every dynamic method call's
/// result -- which, per real JS semantics, eagerly invokes any thenable's
/// `.then`. This executed the query as soon as `.from(t)` returned, with
/// no `WHERE` clause, before `.where(...)` was ever applied.
///
/// Fixed by threading a `chain_intermediate` flag from thaw-llvm's
/// `compile_call_dynamic_method_handle` (`dynamic_host.rs`) through to
/// `thaw_js_call_method_handle_result`: a receiver that is itself a
/// chained `.method()` call is structurally detectable in the HIR (see
/// `lower_dynamic_value_method_call`, thaw-hir's `invocations.rs`, which
/// always lowers a method call's receiver with an expected type of
/// `JsValue`), so only the terminal link in a chain resolves its result --
/// matching real JS semantics, where only an explicit `await`/consumption
/// of the final value would ever settle a pending promise or thenable.
/// See `a_chained_methods_intermediate_thenable_result_is_not_resolved_early`
/// in `registry_modules.rs` for the synthetic, network-free reproduction.
#[test]
fn registry_add_runs_a_real_drizzle_query_builder_chain_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-drizzle-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "drizzle-orm").unwrap();
    // Pinned: better-sqlite3 13 dropped its `prebuild-install` dependency
    // for a packaging scheme the registry doesn't resolve yet ("available
    // targets: none"). 12.11.1 is the last release thaw can fetch a
    // prebuilt `.node` for. (The compiled program below drives the
    // sqlite-proxy driver through an async callback and never loads this
    // addon; it is fetched only so drizzle-orm's peer resolves.)
    thaw_registry::add(&registry, "better-sqlite3@12.11.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { sqliteTable, integer, text } from "drizzle-orm/sqlite-core";
import { drizzle } from "drizzle-orm/sqlite-proxy";
import { eq } from "drizzle-orm";

const users = sqliteTable("users", {
    id: integer(),
    name: text(),
});

async function callback(sql: string, params: JsValue, method: string): Promise<{ rows: number[] }> {
    console.log(sql);
    return { rows: [] };
}

async function main(): Promise<void> {
    const db = drizzle(callback);
    await db.select().from(users).where(eq(users.id, 1));
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
        &["drizzle-orm/sqlite-core".to_string(), "drizzle-orm/sqlite-proxy".to_string()],
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
    let mut lines = stdout.lines();
    let sql = lines.next().unwrap();
    assert!(sql.contains("where"), "expected a WHERE clause: {sql}");
    assert_eq!(lines.next(), None, "query executed more than once: {stdout}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `zod@3.23.0` -- a version-diversity check prompted by the
/// user's own question of whether earlier zod work (this session, against
/// whatever `zod` resolved to as latest -- zod v4) actually generalizes,
/// rather than being overfit to that one observed version. It doesn't:
/// `import { object, string } from "zod"` failed outright (`` `zod` has
/// no export named `object` ``) against v3.
///
/// Root cause: zod v3's actual primitives (`object`, `string`, `number`,
/// ... -- essentially its whole public API) are declared in
/// `lib/types.d.ts` as **bare, non-exported** consts with a **direct**
/// inline function type (no interface involved at all):
/// `declare const objectType: <T extends ZodRawShape>(shape: T, params?)
/// => ZodObject<...>;`, one per primitive, later locally rename-exported
/// (`export { ..., objectType as object, ... };`). This session's earlier
/// `declare const`-export fix (drizzle-orm's `sqliteTable`, commit
/// `c3da7164`) only covered a const whose type is a `TsTypeRef` to a
/// same-file interface with a call signature -- neither a *direct*
/// inline function type, nor a *bare* (non-exported) const reached only
/// through a local rename-export, were covered.
///
/// Fixed by extending exactly that machinery: thaw-bridge's
/// `extract_const_call_signature_decls` gained a `CallableConstSignature::
/// Direct` case (alongside the existing `Interface` one) for a `TsFnType`
/// annotation directly, with a new `lower_dts_fn_type` sibling to
/// `lower_dts_call_signature` synthesizing the `DtsFunction`; thaw-
/// registry's `all_reexported_function_declarations` gained a bare-
/// `Decl::Var` scan feeding its existing `local_declarations` map (the
/// same one a bare `Decl::Fn` already uses), so a same-file rename-export
/// resolves a callable const exactly like it already does a function; and
/// `rename_declared_function` gained `"const "` to its rename-keyword
/// list (previously only `"function "`/`"class "`/`"interface "`), so the
/// renamed binding (`objectType` -> `object`) actually lands in the
/// flattened output under its real, public name.
///
/// `import { z } from "zod"` (zod v3's alternate, namespace-style entry
/// point) needed a separate, follow-on fix -- see
/// `registry_add_builds_a_schema_with_real_zod_v3s_z_namespace_when_enabled`
/// below.
#[test]
fn registry_add_builds_a_schema_with_real_zod_v3_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-zod-v3-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "zod@3.23.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { object, string, number } from "zod";
function main(): void {
    const schema = object({ name: string(), age: number() });
    console.log("ok");
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `zod@3.23.0`'s alternate, namespace-style entry point:
/// `import { z } from "zod"` and `import zod from "zod"` (both
/// `z.object(...)`/`zod.object(...)`), the shape essentially as common in
/// real zod v3 code as the bare-name form the test above covers. Was a
/// separate, still-open gap after that fix: `z` is a self-referential
/// namespace alias (`import * as z from "./external"; export { z };
/// export default z;`) declared in `lib/index.d.ts`, one file *below* the
/// package's own entry point (`index.d.ts`, just `export * from
/// "./lib";`) -- unlike zod v4, whose own entry point declares this
/// alias directly, so it was already visible to `self_referential_
/// namespace_aliases` (thaw-bridge) without any special handling.
/// thaw-registry's flattening only ever pulled individual function/const
/// *declarations* through a wildcard re-export chain, never arbitrary
/// top-level import/export-alias statements from a file reached only
/// transitively, so `z`'s own alias declaration never reached the
/// flattened output at all.
///
/// Fixed with a new `self_referential_namespace_alias_snippets`
/// (thaw-registry's `install.rs`), recursing through wildcard re-exports
/// the same way `collect_namespace_reexports` already does, splicing the
/// exact `import`/`export` snippet text verbatim into the flattened
/// output wherever found (its import source doesn't need to resolve to
/// anything real -- `self_referential_namespace_aliases` only checks the
/// AST shape). `export default z;` also needed
/// `self_referential_namespace_aliases` itself (thaw-bridge) to recognize
/// a *separate* `export default X;` statement as an alias under the
/// implicit name `"default"` -- previously it only matched `export { X as
/// default }`, the form zod v4 happens to use instead.
#[test]
fn registry_add_builds_a_schema_with_real_zod_v3s_z_namespace_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-zod-v3-z-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "zod@3.23.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { z } from "zod";
import zod from "zod";
function main(): void {
    const schema = z.object({ name: z.string(), age: z.number() });
    const schema2 = zod.object({ ok: zod.boolean() });
    console.log("ok");
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_hono_route_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hono-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono@4.13.7").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono } from "hono";
async function main(): Promise<void> {
    const app = new Hono();
    app.get("/", (c) => c.text("Hello Thaw"));
    const response = await app.request("/");
    const body: string = await response.text();
    console.log(response.status);
    console.log(body);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["hono".into()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "200\nHello Thaw\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_fastify_route_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-fastify-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "fastify@5.6.2").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import fastify from "fastify";
async function main(): Promise<void> {
    const app = fastify();
    app.setErrorHandler(async (error, request, reply) => {
        return reply.code(418).send({ message: error.message });
    });
    app.register(async (instance) => {
        instance.get("/users/:id", async (request) => ({ id: request.params.id }));
        instance.post("/echo", async (request) => ({ body: request.body }));
        instance.get("/fail", async () => { throw new Error("expected failure"); });
    }, { prefix: "/api" });
    await app.ready();
    const injected = await app.inject({ method: "GET", url: "/api/users/42" });
    if (Number(injected.statusCode) !== 200) throw "Fastify injection failed";
    const posted = await app.inject({ method: "POST", url: "/api/echo", payload: { name: "Thaw" } });
    if (Number(posted.statusCode) !== 200 || !String(posted.body).includes("Thaw")) throw "Fastify POST injection failed";
    const failed = await app.inject({ method: "GET", url: "/api/fail" });
    if (Number(failed.statusCode) !== 418 || !String(failed.body).includes("expected failure")) throw "Fastify error handler failed";
    await app.listen({ host: "127.0.0.1", port: Number(process.env.PORT) });
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
        &["fastify".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stream = (0..500)
        .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => Some(stream),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("compiled Fastify server did not start");
    stream
        .write_all(b"GET /api/users/42 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200"));
    assert!(response
        .windows(b"{\"id\":\"42\"}".len())
        .any(|bytes| bytes == b"{\"id\":\"42\"}"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_express_route_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-express-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "express@5.1.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import express from "express";
async function main(): Promise<void> {
    const app = express();
    app.use(express.json({ limit: "32b" }));
    app.use(express.urlencoded({ extended: false }));
    app.get("/users/:id", (request, response) => {
        response.json({ id: request.params.id });
    });
    app.post("/json", (request, response) => {
        response.json({ message: request.body.message });
    });
    app.post("/form", (request, response) => {
        response.json({ message: request.body.message });
    });
    app.use((error, _request, response, _next) => {
        response.status(error.status || 500).json({ type: error.type || "internal" });
    });
    const server: JsValue = app.listen(Number(process.env.PORT), "127.0.0.1");
    process.on("SIGTERM", (): void => { server.close(); });
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
        &["express".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        child.try_wait().unwrap().is_none(),
        "async Express server exited before accepting a request"
    );
    let exchange = |request: &[u8]| {
        let mut stream = (0..500)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("compiled Express server did not accept a request");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(request).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        response
    };
    let request = |id: usize| {
        let request = format!(
            "GET /users/{id} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        );
        let response = exchange(request.as_bytes());
        assert!(response.starts_with(b"HTTP/1.1 200"));
        assert!(String::from_utf8_lossy(&response).contains(&format!("{{\"id\":\"{id}\"}}")));
    };
    for id in 0..3 {
        request(id);
    }
    let post = |path: &str, content_type: &str, body: &[u8]| {
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        exchange(&[head.as_bytes(), body].concat())
    };
    let response = post("/json", "application/json", br#"{"message":"json"}"#);
    assert!(response.starts_with(b"HTTP/1.1 200"));
    assert!(String::from_utf8_lossy(&response).contains("{\"message\":\"json\"}"));
    let response = post("/form", "application/x-www-form-urlencoded", b"message=form+value");
    assert!(response.starts_with(b"HTTP/1.1 200"));
    assert!(String::from_utf8_lossy(&response).contains("{\"message\":\"form value\"}"));
    let response = post("/json", "application/json", br#"{"message":}"#);
    assert!(response.starts_with(b"HTTP/1.1 400"));
    let malformed = String::from_utf8_lossy(&response);
    assert!(malformed.contains("entity.parse.failed"), "{malformed}");
    let response = post(
        "/json",
        "application/json",
        br#"{"message":"this body is deliberately larger than thirty-two bytes"}"#,
    );
    assert!(response.starts_with(b"HTTP/1.1 413"));
    assert!(String::from_utf8_lossy(&response).contains("entity.too.large"));
    std::thread::scope(|scope| {
        for id in 3..7 {
            scope.spawn(move || request(id));
        }
    });
    assert!(Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap()
        .success());
    let status = (0..500)
        .find_map(|_| {
            let status = child.try_wait().unwrap();
            if status.is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
            status
        })
        .unwrap_or_else(|| {
            child.kill().unwrap();
            child.wait().unwrap()
        });
    assert!(status.success(), "Express server did not shut down cleanly");
    let _ = std::fs::remove_dir_all(dir);
}

/// Short soak test: Hono's own `app.request(...)` (in-process, no real
/// listener -- matching `registry_add_runs_a_real_hono_route_when_
/// enabled`'s own convention) called repeatedly (a fixed count, not a
/// wall-clock window -- see below) checking for stability (no crash,
/// no wrong response) under sustained load rather than just a single
/// request.
///
/// Deliberately a `for` loop with a plain counter, not a `while` loop
/// with a `Date.now()`-based deadline or a compound `&&`/`||`
/// condition: found, while writing this test, that a `while` loop
/// combining a boolean flag with `Date.now()` in its condition (`while
/// (stable && Date.now() < deadline)`) segfaults after the first
/// iteration, and a *separate* bug makes an `if (a !== x || b !== y)`
/// condition inside such a loop misevaluate even when `a`/`b` print as
/// correct individually -- both reproduced independent of Hono with a
/// minimal synthetic script. Neither is fixed here (out of scope for a
/// test-writing task); flagged to the user as a real, separate finding.
/// The `for`-loop/sequential-`if` shape used here is confirmed not to
/// hit either.
#[test]
fn registry_add_runs_hono_continuously_for_a_short_window_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hono-soak-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono@4.13.7").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono } from "hono";
async function main(): Promise<void> {
    const app = new Hono();
    app.get("/", (c) => c.text("Hello Thaw"));
    let count = 0;
    for (let i = 0; i < 500; i++) {
        const response = await app.request("/");
        const body: string = await response.text();
        if (Number(response.status) !== 200) {
            console.log("bad status at " + i);
            break;
        }
        if (body !== "Hello Thaw") {
            console.log("bad body at " + i);
            break;
        }
        count++;
    }
    console.log("stable:" + count);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["hono".into()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    // The program's own fixed-count `for` loop bounds its runtime -- no
    // external timeout wrapper needed, matching every other
    // process-runs-to-completion test in this file.
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "hono soak run did not exit cleanly: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    let count: usize = stdout
        .trim()
        .strip_prefix("stable:")
        .unwrap_or_else(|| panic!("hono soak run reported an unstable response: {stdout}"))
        .parse()
        .unwrap();
    assert_eq!(count, 500, "expected all 500 requests to succeed");
    let _ = std::fs::remove_dir_all(dir);
}

/// The exact `while`-loop shape the soak test above deliberately avoids
/// (see its own doc comment) -- confirms it's now fixed rather than
/// leaving it permanently worked around. `await app.request(...)`
/// (itself awaited, unlike the soak test's synchronous receiver) then
/// `await response.text()`, checked via a compound `Number(response.
/// status) !== 200 || body !== "..."` condition inside a `while
/// (stable && ...)` loop, used to segfault (a dangling string pointer
/// read by `strcmp`, confirmed via `gdb`) or silently misevaluate the
/// condition on the loop's *second* iteration onward -- the first
/// iteration, and a `for` loop with an otherwise-identical body, never
/// reproduced it.
///
/// Root cause: `response`/`body`, declared directly inside the loop
/// body (redeclared -- the same `Let` statement recompiled -- every
/// iteration), are correctly recognized as async-frame locals (durable
/// storage meant to survive the suspend/resume boundary between
/// iterations), but `HirStmt::Let`'s ordinary compilation
/// (`crates/thaw-llvm/src/hir_codegen/statements.rs`) unconditionally
/// allocated a *new*, ordinary stack cell and rebound `self.
/// variables[name]` to it regardless -- clobbering the frame-backed
/// binding `bind_async_frame_locals` had just installed moments
/// earlier. That new cell is hoisted into the *current* physical
/// function's entry block, valid only for that one invocation of the
/// async coroutine's `resume` function -- not across the separate
/// invocation the next iteration's own suspend/resume causes. The `&&`/
/// `||` operators' closure-based desugaring (`lower_logical_expr`/
/// `wrap_call_argument_bindings`, `crates/thaw-hir/src/lower/
/// expressions/coercions.rs` and `.../invocations/dynamic_values.rs`)
/// captures `response`/`body` into that closure; by the second
/// iteration the captured pointer is dangling, reading whatever now
/// occupies that stack slot -- silently wrong values, or a crash,
/// depending on what that happens to be. A local declared *before* the
/// loop (its own `Let` never re-executes) was unaffected, which is why
/// `deadline` in the original repro worked fine while `response`/`body`
/// didn't.
///
/// Fixed by having `HirStmt::Let` reuse the existing frame cell (just
/// `build_store` into it) instead of allocating a fresh one, whenever
/// the name being declared is already bound to one.
#[test]
fn registry_add_runs_a_real_hono_route_repeatedly_through_a_while_loop_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-hono-while-loop-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono@4.13.7").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono } from "hono";
async function main(): Promise<void> {
    const app = new Hono();
    app.get("/", (c) => c.text("Hello Thaw"));
    const deadline = Date.now() + 3000;
    let count = 0;
    let ok = true;
    while (ok && count < 4 && Date.now() < deadline) {
        const response = await app.request("/");
        const body: string = await response.text();
        if (Number(response.status) !== 200 || body !== "Hello Thaw") {
            ok = false;
        } else {
            count++;
        }
    }
    console.log(ok ? "stable:" + count : "unstable at request " + count);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["hono".into()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "hono while-loop run did not exit cleanly: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "stable:4\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Short soak test: a real, listening Fastify server hit with
/// sequential HTTP requests over a fixed wall-clock window, then
/// cleanly shut down -- checks the compiled binary stays alive and
/// correct under sustained load, not just for one request.
#[test]
fn registry_add_runs_fastify_continuously_for_a_short_window_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-fastify-soak-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "fastify@5.6.2").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import fastify from "fastify";
async function main(): Promise<void> {
    const app = fastify();
    app.get("/ping", async () => ({ ok: true }));
    await app.listen({ host: "127.0.0.1", port: Number(process.env.PORT) });
    process.on("SIGTERM", (): void => { app.close(); });
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
        &["fastify".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (0..500)
        .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => Some(stream),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("compiled Fastify server did not start");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut count = 0usize;
    while Instant::now() < deadline {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(
            response.starts_with(b"HTTP/1.1 200"),
            "request {count} failed: {}",
            String::from_utf8_lossy(&response)
        );
        assert!(child.try_wait().unwrap().is_none(), "Fastify server died during the soak run");
        count += 1;
    }
    assert!(count > 5, "expected several requests in the 3s window, only got {count}");
    assert!(Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap()
        .success());
    let status = (0..500)
        .find_map(|_| {
            let status = child.try_wait().unwrap();
            if status.is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
            status
        })
        .unwrap_or_else(|| {
            child.kill().unwrap();
            child.wait().unwrap()
        });
    assert!(status.success(), "Fastify server did not shut down cleanly after the soak run");
    let _ = std::fs::remove_dir_all(dir);
}

/// Short soak test: a real, listening Express server hit with
/// sequential HTTP requests over a fixed wall-clock window, then
/// cleanly shut down -- same shape as the Fastify soak test above.
#[test]
fn registry_add_runs_express_continuously_for_a_short_window_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-express-soak-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "express@5.1.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import express from "express";
async function main(): Promise<void> {
    const app = express();
    app.get("/ping", (request, response) => {
        response.json({ ok: true });
    });
    const server: JsValue = app.listen(Number(process.env.PORT), "127.0.0.1");
    process.on("SIGTERM", (): void => { server.close(); });
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
        &["express".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (0..500)
        .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => Some(stream),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("compiled Express server did not start");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut count = 0usize;
    while Instant::now() < deadline {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(
            response.starts_with(b"HTTP/1.1 200"),
            "request {count} failed: {}",
            String::from_utf8_lossy(&response)
        );
        assert!(child.try_wait().unwrap().is_none(), "Express server died during the soak run");
        count += 1;
    }
    assert!(count > 5, "expected several requests in the 3s window, only got {count}");
    assert!(Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap()
        .success());
    let status = (0..500)
        .find_map(|_| {
            let status = child.try_wait().unwrap();
            if status.is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
            status
        })
        .unwrap_or_else(|| {
            child.kill().unwrap();
            child.wait().unwrap()
        });
    assert!(status.success(), "Express server did not shut down cleanly after the soak run");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_processes_a_real_hono_sharp_image_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-hono-sharp-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono@4.13.7").unwrap();
    thaw_registry::add(&registry, "@hono/node-server@1.19.9").unwrap();
    thaw_registry::add(&registry, "sharp@0.35.4").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono } from "hono";
import { serve } from "@hono/node-server";
import sharp from "sharp";
import { Buffer } from "node:buffer";
const app = new Hono();
app.onError((error, c) => c.text(error.message, 500));
app.post("/images", async (c) => {
    const input = Buffer.from(await c.req.arrayBuffer());
    const output = await sharp(input).resize(512, 512).webp().toBuffer();
    return c.body(output, 200, { "Content-Type": "image/webp" });
});
function main(): void {
    serve({ fetch: app.fetch, port: Number(process.env.PORT) });
}"#,
    )
    .unwrap();
    let build_started = Instant::now();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &[
            "hono".into(),
            "@hono/node-server".into(),
            "sharp".into(),
        ],
    )
    .unwrap();
    record_acceptance_metrics("hono-sharp", build_started.elapsed(), &output);
    std::fs::remove_dir_all(&registry).unwrap();

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .current_dir(&dir)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stream = (0..500)
        .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => Some(stream),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("compiled Hono + sharp server did not start");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"#;
    stream
        .write_all(
            format!(
                "POST /images HTTP/1.1\r\nHost: localhost\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{svg}",
                svg.len()
            )
            .as_bytes(),
        )
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    let mut stderr = String::new();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert!(
        response.starts_with(b"HTTP/1.1 200"),
        "{}\n{stderr}",
        String::from_utf8_lossy(&response),
    );
    assert!(String::from_utf8_lossy(&response).contains("Content-Type: image/webp"));
    assert!(response.windows(4).any(|bytes| bytes == b"RIFF"));
    assert!(response.windows(4).any(|bytes| bytes == b"WEBP"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_and_serves_hono_react_prisma_postgres_when_enabled() {
    let Ok(database_url) = std::env::var("THAW_POSTGRES_URL") else {
        return;
    };
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/hono-react-prisma-board")
        .canonicalize()
        .unwrap();
    assert!(npm_run_command("generate:postgres", &project)
        .env("DATABASE_URL", &database_url)
        .status()
        .unwrap()
        .success());
    assert!(npm_run_command("db:push:postgres", &project)
        .env("DATABASE_URL", &database_url)
        .status()
        .unwrap()
        .success());

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-hono-react-prisma-postgres-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    std::fs::create_dir_all(&dir).unwrap();
    thaw_registry::add(&registry, "hono@4.13.7").unwrap();
    thaw_registry::add(&registry, "@hono/node-server@1.19.9").unwrap();
    thaw_registry::add_installed_root(
        &registry,
        &project.join("node_modules"),
        "@prisma/client",
    )
    .unwrap();
    let assets = build_vite_project(&project).unwrap();
    let output = dir.join("app");
    let build_started = Instant::now();
    build_with_native_mode(
        &project.join("server.ts"),
        &output,
        &[],
        &[],
        &[],
        &registry,
        &[
            "hono".into(),
            "@hono/node-server".into(),
            "@prisma/client".into(),
        ],
        false,
        Some(&assets),
        true,
    )
    .unwrap();
    record_acceptance_metrics(
        "hono-react-prisma-postgres",
        build_started.elapsed(),
        &output,
    );
    std::fs::remove_dir_all(&registry).unwrap();

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .current_dir(&dir)
        .env("DATABASE_URL", &database_url)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let request = |method: &str, path: &str, body: &str| {
        let mut stream = (0..500)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("compiled Hono + React + Prisma server did not start");
        stream
            .write_all(
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    assert!(request("GET", "/", "").contains("<title>Thaw掲示板</title>"));
    assert!(request("GET", "/api/posts", "").contains(r#"{"posts":[]}"#));
    let created = request(
        "POST",
        "/api/posts",
        r#"{"author":"Yuu","message":"PostgreSQL"}"#,
    );
    assert!(created.starts_with("HTTP/1.1 201"), "{created}");
    assert!(created.contains(r#""author":"Yuu""#), "{created}");
    let listed = request("GET", "/api/posts", "");
    assert!(listed.contains(r#""message":"PostgreSQL""#), "{listed}");

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_hapi_route_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hapi-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "@hapi/hapi@21.4.3").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import * as Hapi from "@hapi/hapi";
async function main(): Promise<void> {
    const server = Hapi.server({ port: 0 });
    server.route({ method: "GET", path: "/", handler: (request, h) => h.response(request.path) });
    const response = await server.inject("/");
    console.log(response.statusCode);
    console.log(response.payload);
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
        &["@hapi/hapi".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "200\n/\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_infers_a_real_commander_action_callback_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-commander-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "commander@14.0.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Command } from "commander";
function main(): void {
    const program = new Command();
    program.argument("<name>");
    program.action((name) => console.log(name));
    program.parse(["node", "app", "Thaw"]);
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
        &["commander".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "Thaw\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_real_pg_pool_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!("thaw-cli-real-pg-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "pg@8.16.3").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Pool } from "pg";
async function main(): Promise<void> {
    const pool = new Pool({ host: "127.0.0.1", port: __PORT__, user: "thaw", database: "thaw", connectionTimeoutMillis: 500 });
    try {
        await pool.connect();
    } catch (error) {
        console.log("closed");
    }
    await pool.end();
}"#
        .replace("__PORT__", &port.to_string()),
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["pg".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let peer = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    drop(stream);
                    return true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    });
    let result = Command::new(&output).output().unwrap();
    assert!(peer.join().unwrap(), "pg Pool did not reach the TCP peer");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "closed\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_queries_postgres_with_real_pg_when_enabled() {
    let database_url = std::env::var("THAW_POSTGRES_URL").ok();
    if database_url.is_none()
        && std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1")
    {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-real-pg-query-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "pg@8.16.3").unwrap();
    thaw_registry::add(&registry, "hono@4.13.7").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Client, Pool } from "pg";
import { Hono } from "hono";
async function main(): Promise<void> {
    const client = new Client({ connectionString: process.env.DATABASE_URL });
    await client.connect();
    const result: JsValue = await client.query("SELECT 42::int AS value");
    console.log(result.rows[0].value);
    await client.end();

    const pool = new Pool({ connectionString: process.env.DATABASE_URL });
    const pooled = await pool.connect();
    const parameterized: JsValue = await pooled.query("SELECT $1::int AS value", [43]);
    console.log(parameterized.rows[0].value);
    pooled.release();

    const app = new Hono();
    app.get("/value", async (c) => {
        const queried: JsValue = await pool.query("SELECT $1::int AS value", [44]);
        return c.json({ value: Number(readDynamicValue(queried.rows[0].value)) });
    });
    const response: JsValue = await app.request("/value");
    const body: string = await response.text();
    console.log(body);
    await pool.end();
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
        &["pg".into(), "hono".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let Some(database_url) = database_url else {
        let _ = std::fs::remove_dir_all(dir);
        return;
    };
    let result = Command::new(&output)
        .env("DATABASE_URL", database_url)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "42\n43\n{\"value\":44}\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}
