/// One of the Phase-4 validation targets (see [[project_thaw_overview]]):
/// a real, `thaw registry add`-fetched zod builds and validates schemas
/// end-to-end, including a fully inline method chain
/// (`z.string().min(2).max(10).safeParse(...)`, no intermediate binding
/// at all) -- exercises `a_method_can_be_chained_directly_onto_another_
/// methods_call_result`'s fix against the real package, not just a
/// synthetic double. Pins down the whole "make zod work" arc from
/// [[project_npm_interop_gaps_2]] as a permanent regression test instead
/// of leaving it as one-off manual verification nobody would notice
/// regress.
///
/// Also covers `.refine()`/`.transform()` -- a real compiled (native)
/// closure passed as a dynamic-call argument -- see thaw-cli's own
/// `a_native_closure_can_be_passed_as_an_argument_to_a_dynamic_method_call`
/// for the synthetic, network-free version of the same mechanism -- and
/// `.pipe(z.string().min(3))`, a dynamic-call argument that's itself a
/// method call chained off a `JsValue` receiver with no intermediate
/// binding at all (see `a_dynamic_call_argument_that_is_itself_a_
/// chained_method_call_stays_live` for the synthetic version) -- and
/// `.superRefine((val, ctx) => { ctx.addIssue(...); })`, a `JsValue`-
/// typed callback parameter whose own method call reenters the dynamic-
/// call machinery from inside a native callback (see thaw-cli's own
/// `a_superrefine_shaped_native_callback_with_a_jsvalue_context_
/// parameter_works` for the synthetic version, and its own doc comment
/// for the four gaps this exercises together).
#[test]
fn registry_add_validates_zod_schemas_end_to_end_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-zod-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "zod").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import * as z from "zod";
function main(): void {
    console.log(z.string().min(2).max(10).safeParse("hello").success);
    console.log(z.string().min(2).max(10).safeParse("h").success);
    const schema = z.object({ name: z.string(), age: z.number() });
    console.log(schema.safeParse({ name: "Alice", age: 30 }).success);
    console.log(schema.safeParse({ name: "Alice", age: "thirty" }).success);
    console.log(z.number().refine((n: number) => n > 0, { message: "must be positive" }).safeParse(5).success);
    console.log(z.number().refine((n: number) => n > 0, { message: "must be positive" }).safeParse(-5).success);
    console.log(JSON.stringify(z.string().transform((s: string) => s.length).parse("hello")));
    console.log(z.string().pipe(z.string().min(3)).safeParse("hi").success);
    console.log(z.string().pipe(z.string().min(3)).safeParse("hello").success);
    const withCtx: JsValue = z.object({ a: z.number(), b: z.number() }).superRefine((val: Json, ctx: JsValue) => {
        if (Number(val.a) > Number(val.b)) {
            ctx.addIssue({ code: "custom", message: "a must be <= b" });
        }
    });
    console.log(withCtx.safeParse({ a: 1, b: 2 }).success);
    console.log(withCtx.safeParse({ a: 3, b: 2 }).success);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["zod".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\nfalse\ntrue\nfalse\ntrue\nfalse\n5\nfalse\ntrue\ntrue\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A real, fetched dayjs -- another Phase-4 validation target -- parses
/// and formats dates through both a single-hop chain (`dayjs(date).
/// format(...)`) and a double-hop one (`dayjs(date).add(...).format(
/// ...)`), fully inline with no intermediate `const` anywhere. Both
/// used to fail to build ("unsupported member call target") before this
/// session's two chained-call fixes; only UTC-based formatting is
/// asserted (never a local-time token) so this doesn't depend on the
/// host's time zone.
#[test]
fn registry_add_formats_dates_with_dayjs_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-dayjs-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "dayjs").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import dayjs from "dayjs";
function main(): void {
    console.log(dayjs("2024-01-15T00:00:00Z").format("YYYY-MM-DD"));
    console.log(dayjs("2024-01-15T00:00:00Z").add(10, "day").format("YYYY-MM-DD"));
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
        "2024-01-15\n2024-01-25\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A real, fetched hono -- the last of the Phase-4 validation targets
/// exercised this session -- constructs its `Hono` class via `new`.
/// Pins the constructor half of [[project_npm_interop_gaps_2]]'s hono
/// arc (commit `9addc760`) against the real package. Deliberately scoped
/// to construction only, not route registration -- see `registry_add_
/// routes_and_serves_a_real_hono_app_when_enabled` below for that half,
/// which turned out not to need the separate callable-interface-typing
/// feature this comment used to describe: hono's real, flattened
/// `package.d.ts` never actually extracts `HonoBase`'s own methods
/// (`.get`/`.post`/`.fetch`/`.request`), so every call to them already
/// goes through the untyped/dynamic dispatch path built for zod, which
/// doesn't care what `Handler`'s own declared type is at all.
#[test]
fn registry_add_constructs_a_hono_app_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hono-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono } from "hono";
function main(): void {
    const app = new Hono();
    console.log("hono app created");
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
        "hono app created\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The route-registration half of the real hono arc left unattempted by
/// the constructor-only test above: `app.get(path, handler)` followed by
/// `app.request(path)` (hono's own synchronous testing helper) reading
/// `.status`/`.text()` off the resulting real `Response`.
///
/// This used to read `.status` back as `undefined` with no error at all
/// -- the handler's `return c.text(...)` silently lowered its dynamic
/// method call through the untyped/JSON-decoding dispatch instead of
/// keeping the real handle, so hono's router received a content-free
/// `{}` snapshot in place of the actual `Response` object. See the
/// synthetic, network-free reproduction and full root-cause writeup at
/// `a_dynamic_callback_argument_returning_a_jsvalue_keeps_it_live_even_
/// when_unannotated` in `registry_modules.rs` -- this test just confirms
/// the same fix holds against the real, unmodified npm package.
///
/// Exercises both an unannotated block-body handler and an unannotated
/// implicit-return expression-body handler, matching how real hono
/// handlers are actually written (`Handler`'s own declared return type
/// is never surfaced through the flattened `.d.ts`, so nobody writing
/// real hono code annotates a handler's return type at all). Also reads
/// the response body back as a plain `string` (`await res.text()`, not
/// `JsValue`) -- a separate, narrower gap noted while finishing this
/// arc and fixed alongside it: `coerce_to_declared` had no path for
/// decoding a dynamic call's default `Json` result into a declared
/// scalar type at all (see `a_dynamic_method_calls_json_result_decodes_
/// into_a_declared_scalar_type` in `registry_modules.rs` for the
/// synthetic, network-free reproduction).
#[test]
fn registry_add_routes_and_serves_a_real_hono_app_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-hono-route-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "hono").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Hono, Context } from "hono";
async function main(): Promise<void> {
    const appA = new Hono();
    appA.get('/', (c: Context) => { return c.text('hello world'); });
    const resA: JsValue = appA.request('/');
    console.log(resA.status);

    const appB = new Hono();
    appB.get('/', (c: JsValue) => c.text('hello again'));
    const resB: JsValue = appB.request('/');
    console.log(resB.status);
    const bodyB: string = await resB.text();
    console.log(bodyB);
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
        "200\n200\nhello again\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched uuid -- the first breadth-first bug-hunting target after
/// zod/dayjs/hono all closed out (per [[project_thaw_overview]]'s Phase-4
/// roadmap). `v4`/`v1`/`v3`/`v5`/`v6`/`v7`/`validate`/`version` all build
/// and run correctly with no changes needed (both real and synthetic
/// `v3`/`v5` outputs were cross-checked against a direct `node` run of
/// uuid's own bundle, byte-for-byte identical).
///
/// `parse(id)` (returns `NonSharedArrayBuffer`) piped straight into
/// `stringify(bytes)` (parameter `Uint8Array`, second parameter `offset`
/// omitted) crashed LLVM's own module verifier outright -- both
/// unresolved reference types classify the same "unclassified npm type"
/// way thaw-bridge falls back to for real, but land on *opposite* sides
/// of the return-value/parameter divide (`JsValue` vs `Json`), and
/// `stringify`'s own omitted-second-parameter dispatch routes the call
/// through an *ordinary* compiled function call, not a dynamic-call
/// intrinsic -- see the synthetic, network-free reproduction and full
/// root-cause writeup at `a_jsvalue_returning_functions_result_passed_
/// into_another_functions_json_parameter_works` in `registry_modules.rs`.
/// This test just confirms the same fix holds against the real,
/// unmodified npm package.
///
/// One known, deliberately-unattempted gap found along the way, scoped
/// as a separate, larger feature rather than a narrow bug: uuid's
/// `NIL`/`MAX` constant exports (`export { default as NIL } from
/// './nil.js'`, where the target file's default export is a bare
/// `declare const`, not a function) -- thaw's whole package-export
/// pipeline (thaw-bridge's parser, thaw-cli's shim generation) only ever
/// recognizes a function or class/interface export, never a plain data
/// constant, so this would need a genuinely new export kind threaded
/// through several layers, not a flattening fix alone.
///
/// `v4`'s own buffer-output overload (`v4<TBuf extends Uint8Array =
/// Uint8Array>(options, buf, offset?): TBuf`) -- previously unreachable,
/// always resolving to `v4`'s first (`buf?: undefined`) overload no
/// matter what a real call passed -- is now fixed (registry Fallback
/// functions pick an overload by real call-site arity/argument-type
/// scoring, the same mechanism external class methods already used; see
/// `fallback_function_overload_is_picked_by_call_site_arity` in
/// `registry_modules.rs` for the synthetic reproduction) and exercised
/// below.
#[test]
fn registry_add_generates_and_parses_uuids_with_real_uuid_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-uuid-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "uuid").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { v4, validate, version, parse, stringify } from "uuid";
function main(): void {
    const id: string = v4();
    console.log(validate(id));
    console.log(version(id));

    const ns = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
    const bytes = parse(ns);
    console.log(bytes.length);
    const roundTripped: string = stringify(bytes);
    console.log(roundTripped);

    const filled: JsValue = v4(undefined, bytes as JsValue);
    console.log(filled);
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
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("4"));
    assert_eq!(lines.next(), Some("16"));
    assert_eq!(lines.next(), Some("6ba7b810-9dad-11d1-80b4-00c04fd430c8"));
    // If the buffer-output overload had stayed unreachable (always
    // resolving to `v4`'s first, `string`-returning overload instead),
    // this call wouldn't even have compiled -- `const filled: JsValue =
    // v4(...)` would be a type mismatch against a `string` result.
    let filled_line = lines.next().expect("v4's buffer overload result");
    let filled_bytes: Vec<&str> = filled_line.split(',').collect();
    assert_eq!(
        filled_bytes.len(),
        16,
        "expected a 16-byte filled buffer: {filled_line}"
    );
    assert_ne!(
        filled_line, "107,167,184,16,157,173,17,209,128,180,0,192,79,212,48,200",
        "buffer wasn't actually filled with random bytes by the buffer-output overload"
    );
    assert_eq!(lines.next(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `uuid@8.3.2` -- a legacy major version predating the
/// currently-fixed latest uuid (the test above), with a completely
/// different `.d.ts` export shape: `uuid@8.3.2` itself ships no `.d.ts`
/// at all (types come from the separately-versioned `@types/uuid`),
/// whose real `index.d.mts` is `import uuid from "./index.js"; export
/// import v1 = uuid.v1; export import v4 = uuid.v4; export import
/// validate = uuid.validate; ...` -- a TS import-equals declaration whose
/// module reference is a qualified *entity name* (`uuid.v1`, a property
/// access into an already-imported value), not a `require(...)` call.
/// thaw-registry's flattening had no code recognizing this AST shape at
/// all, so none of uuid v8's functions were reachable.
///
/// Root cause, two parts: (1) nothing in `install.rs` matched
/// `TsModuleRef::TsEntityName` at all (only `TsExternalModuleRef`, the
/// `require(...)` shape `import_equals_targets` already handles) -- fixed
/// by a new loop resolving the qualifier (`uuid`) via the existing
/// `named_import_targets`, then reusing `reexported_function_
/// declarations` to look the member (`v1`) up in `uuid`'s own target
/// file exactly like a named re-export already does. (2) even once
/// found, `uuid.v1`'s own declared type (`export const v1: v1;`) is
/// typed through a *local, unexported* type-alias chain
/// (`type v1 = v1Buffer & v1String;`, each side itself a further local
/// alias resolving to a direct function type) -- a shape neither
/// `callable_const_declaration_snippet` (thaw-registry, text inclusion)
/// nor `extract_const_call_signature_decls` (thaw-bridge, classification)
/// recognized, both only handling a same-file call-signature interface or
/// a *direct* inline function type. Fixed on the thaw-registry side by
/// carrying the const's own snippet together with *every* type alias/
/// interface declared in the same file (harmless when unrelated -- `.d.ts`
/// type declarations are erasable); on the thaw-bridge side by a new
/// `resolve_local_callable_fn_types`, following a local alias chain and
/// unwrapping an intersection into each of its own operands, yielding one
/// `DtsFunction` per resolved overload (`v1Buffer`'s generic
/// buffer-output overload and `v1String`'s simple string-returning one),
/// the same "first overload wins" convention already documented as a
/// separate, scoped-out limitation for the latest uuid's own `v4`
/// (confirmed present here too, for the exact same reason -- not
/// attempted here either).
#[test]
fn registry_add_reexports_uuid_v8s_import_equals_functions_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-uuid-v8-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "uuid@8.3.2").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { validate, version, parse, stringify } from "uuid";
function main(): void {
    const ns = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
    console.log(validate(ns));
    console.log(version(ns));
    const bytes = parse(ns);
    console.log(bytes.length);
    const roundTripped: string = stringify(bytes);
    console.log(roundTripped);
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
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("1"));
    assert_eq!(lines.next(), Some("16"));
    assert_eq!(lines.next(), Some("6ba7b810-9dad-11d1-80b4-00c04fd430c8"));
    assert_eq!(lines.next(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Real, fetched `lodash@4.17.21` -- both entry-point shapes
/// (`import _ from "lodash"` and `import * as _ from "lodash"`), plus a
/// named import of a function with an *optional* callback parameter
/// (`filter`). Found via the earlier lodash `default`-import fix's own
/// verification against real lodash's full `.d.ts`: even
/// `import * as _ from "lodash"; _.chunk(...)` (calling a function with
/// no callback parameter at all) crashed with "unsupported dynamic
/// object field Function(...)".
///
/// Root cause: an *optional* callback parameter (real example: lodash's
/// `filter(collection: string | null | undefined, predicate?:
/// StringIterator<boolean>): string[];`) classifies as `Native(Optional
/// (Function(...)))`, a different `Native` variant from the bare
/// `Native(Function(...))` case this session's earlier closure-as-
/// plain-argument fix (commit `170f6a23`) covered, so it fell through
/// `typed_dynamic_declaration`'s (thaw-cli's `shims.rs`) widening match
/// arm unwidened. Separately, `typed_dynamic_bare_alias` (the wrapper
/// generated under a function's bare name) had its own, independent,
/// entirely unwidened parameter-type computation -- so a plain named
/// import of any function with such a parameter crashed the moment it
/// was called with the parameter omitted, regardless of the const-
/// export-shape fix above. Both fixed the same way -- widened to `Json`
/// for the non-`napi` backend, matching the existing bare-`Function`
/// case -- and `typed_dynamic_bare_alias` additionally needed `napi`
/// threaded through as a new parameter (it previously had no napi-
/// awareness at all).
#[test]
fn registry_add_builds_and_calls_real_lodash_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-lodash-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "lodash@4.17.21").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import _ from "lodash";
import * as ns from "lodash";
import { every, filter, find, map, reduce, some, sortBy } from "lodash";
function main(): void {
    let sortTotal: number = 0;
    let objectSortTotal: number = 0;
    console.log(_.chunk([1, 2, 3, 4], 2).length);
    console.log(_.capitalize("hello"));
    console.log(ns.chunk([1, 2, 3, 4], 2).length);
    console.log(_.filter("hello").length);
    console.log(map([1, 2, 3], value => value * 2).length);
    console.log(filter([1, 2, 3], value => value > 1).length);
    console.log(reduce([1, 2, 3], (sum, value) => sum + value, 0));
    console.log(find([1, 2, 3], value => value > 1));
    console.log(some([1, 2, 3], value => value === 2));
    console.log(every([1, 2, 3], value => value > 0));
    const sorted = sortBy([3, 1, 2], value => { sortTotal = sortTotal + value; return value; });
    console.log(sorted[0]);
    console.log(sorted.join(','));
    console.log(sortTotal);
    const sortedObject = sortBy({ a: 4, b: 2 }, value => { objectSortTotal = objectSortTotal + value; return value; });
    console.log(sortedObject[0]);
    console.log(sortedObject.join(','));
    console.log(objectSortTotal);
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
    let stdout = String::from_utf8_lossy(&result.stdout);
    let mut lines = stdout.lines();
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("Hello"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("5"));
    assert_eq!(lines.next(), Some("3"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("6"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("1"));
    assert_eq!(lines.next(), Some("1,2,3"));
    assert_eq!(lines.next(), Some("6"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("2,4"));
    assert_eq!(lines.next(), Some("6"));
    assert_eq!(lines.next(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// `picocolors`' whole API is a single `declare const picocolors: Colors
/// & { createColors: ... }` whose functions come back across the
/// Fallback boundary as `Json`. `pc.dim("x") + pc.underline("y")` --
/// concatenating two of those results -- used to fail lowering with
/// `arithmetic requires F64 operands, got Json and Json`; `+` on a
/// `Json` operand now falls back to `String(x) + String(y)` the way JS
/// would for a non-numeric `+`.
#[test]
fn registry_add_concatenates_dynamic_results_with_picocolors_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-picocolors-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "picocolors@1.1.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import pc from "picocolors";
function main(): void {
    console.log(pc.green("ok"));
    console.log(pc.bold(pc.red("bad")));
    console.log(pc.dim("x") + pc.underline("y"));
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["picocolors".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Not a TTY, so picocolors emits no ANSI escapes -- just the text,
    // and the `+` produced a real string join rather than a build error.
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\nbad\nxy\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_and_calls_real_chalk_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-chalk-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "chalk@5.4.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import chalk from "chalk";
function main(): void {
    console.log(chalk.red.bold("hello"));
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
    assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_real_safe_buffer_string_conversions_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-safe-buffer-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "safe-buffer@5.2.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { Buffer } from "safe-buffer";
function main(): void {
    console.log(Buffer.from("雪").toString("hex"));
    console.log(Buffer.from("e99baa", "hex").toString());
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
        &["safe-buffer".to_string()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "e99baa\n雪\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `semver`'s `.d.ts` returns `string | null` from `valid`/`coerce`/
/// `diff` and unions like `string | number` elsewhere -- exercises
/// thaw's `Nullable`/`Union` truthiness (`if (v)`), string coercion
/// (`"x: " + v`), `??`, and flow narrowing: `if (v) { v.method() }` and
/// `if (!v) return;` narrow `v` from `string | null` to `string`.
#[test]
fn registry_add_uses_real_semver_nullable_and_union_returns_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-semver-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "semver@7.6.3").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import semver from "semver";
function normalize(raw: string): string {
    const v = semver.valid(raw);
    if (!v) { return "invalid"; }
    return v.toUpperCase();
}
function shout(raw: string): string {
    // `x && x.method()` on a `string | null` -> `string | null`.
    const v = semver.valid(raw);
    const loud = v && v.toUpperCase();
    return loud ? loud : "-";
}
function labelled(raw: string): string {
    const v = semver.valid(raw);
    // `typeof x === "string"` narrows `string | null` -> `string`.
    if (typeof v === "string") { return "v=" + v.length; }
    // `x || default` on a nullable.
    return v || "missing";
}
function main(): void {
    const good = semver.valid("1.2.3");
    console.log(good ? "valid: " + good : "invalid");
    const bad = semver.valid("nope");
    console.log(bad ? "valid: " + bad : "invalid");
    const coerced = semver.coerce("v2");
    console.log(coerced ? "coerced" : "no");
    console.log(semver.gt("1.2.3", "1.2.0"));
    console.log(semver.major("2.5.9"));
    console.log(semver.diff("1.2.3", "2.0.0") ?? "none");
    const cleaned = semver.clean(" =1.4.7 ");
    if (cleaned) { console.log("cleaned:" + cleaned.length); }
    console.log(normalize("1.2.3"));
    console.log(normalize("bogus"));
    console.log(shout("3.1.4"));
    console.log(shout("bad"));
    console.log(labelled("9.9.9"));
    console.log(labelled("nope"));
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
        "valid: 1.2.3\ninvalid\ncoerced\ntrue\n2\nmajor\ncleaned:5\n1.2.3\ninvalid\n3.1.4\n-\nv=5\nmissing\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `debug` -- its default export's `.d.ts` type is `debug.Debug & {
/// ... }`, an intersection of a *callable* interface (`(ns): Debugger`)
/// with a plain property bag. `createDebug(ns)` returns a live
/// `Debugger`, and calling that -- `log("msg %s", x)` -- is the whole
/// point of the package.
#[test]
fn registry_add_runs_a_real_debug_logger_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-debug-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "debug@4.3.7").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import createDebug from "debug";
function main(): void {
    // The config API on the callable default export -- a function-typed
    // property signature (`enable: (ns) => void`) on `debug.Debug`, on
    // top of its `(ns): Debugger` call signature.
    createDebug.enable("app:*");
    console.log(createDebug.enabled("app:db"));
    console.log(createDebug.enabled("other:x"));
    const log = createDebug("app:db");
    const other = createDebug("app:cache");
    log("query %s took %d ms", "SELECT 1", 12);
    other("miss");
    console.log("done");
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    // `createDebug.enable("app:*")` runs from code, so the namespaces are
    // active regardless of the DEBUG env var. `enabled(...)` reflects
    // that (`true` for a matching namespace, `false` otherwise), and both
    // loggers fire to stderr. debug writes only to stderr, so stdout is
    // just our three `console.log`s.
    for debug_env in [None, Some("nothing:here")] {
        let mut command = Command::new(&output);
        match debug_env {
            Some(value) => command.env("DEBUG", value),
            None => command.env_remove("DEBUG"),
        };
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            "true\nfalse\ndone\n"
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("app:db query SELECT 1 took 12 ms"),
            "stderr: {stderr}"
        );
        assert!(stderr.contains("app:cache miss"), "stderr: {stderr}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// `mustache` -- its `.d.ts` is `import * as m from "./index.js";
/// export default m;` plus two option interfaces: no usable function
/// types at all (the real API is JSDoc in a `.js` file). thaw synthesizes
/// a `default` value export bound to the runtime module object, so
/// `import M from "mustache"; M.render(tpl, view)` works through the
/// dynamic method-call path.
#[test]
fn registry_add_renders_a_real_mustache_template_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-mustache-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "mustache@4.2.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import Mustache from "mustache";
function main(): void {
    const out: string = Mustache.render(
        "{{greeting}}, {{name}}!",
        JSON.parse("{\"greeting\":\"Hello\",\"name\":\"thaw\"}")
    );
    console.log(out);
    console.log(Mustache.escape("<a>&\"'"));
    const tokens = Mustache.parse("Hi {{x}}");
    console.log(JSON.stringify(tokens).length > 2 ? "parsed" : "empty");
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
        "Hello, thaw!\n&lt;a&gt;&amp;&quot;&#39;\nparsed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `pluralize` -- a callable default export that also has methods
/// (`pluralize.singular`, `pluralize.isPlural`), plus an optional numeric
/// second argument.
#[test]
fn registry_add_runs_real_pluralize_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-pluralize-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "pluralize@8.0.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import pluralize from "pluralize";
function main(): void {
    console.log(pluralize("cat"));
    console.log(pluralize("cat", 1));
    console.log(pluralize("cat", 3));
    console.log(pluralize.singular("boxes"));
    console.log(pluralize.isPlural("dogs"));
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
        "cats\ncat\ncats\nbox\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `deepmerge` -- a recursive object merge crossing the dynamic bridge
/// and coming back as `Json`.
#[test]
fn registry_add_runs_real_deepmerge_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-deepmerge-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "deepmerge@4.3.1").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import deepmerge from "deepmerge";
function main(): void {
    const merged: Json = deepmerge(
        JSON.parse("{\"a\":1,\"b\":{\"x\":1}}"),
        JSON.parse("{\"b\":{\"y\":2},\"c\":3}")
    );
    console.log(JSON.stringify(merged));
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
        "{\"a\":1,\"b\":{\"x\":1,\"y\":2},\"c\":3}\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `qs`, `clsx`, and `slugify` -- object/array-shaped arguments and
/// options bags through the dynamic bridge, and query-string round trips.
#[test]
fn registry_add_runs_real_qs_clsx_and_slugify_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-qs-clsx-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "qs@6.13.0").unwrap();
    thaw_registry::add(&registry, "clsx@2.1.1").unwrap();
    thaw_registry::add(&registry, "slugify@1.6.6").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import qs from "qs";
import clsx from "clsx";
import slugify from "slugify";
function main(): void {
    console.log(qs.stringify(JSON.parse("{\"a\":\"1\",\"b\":\"2\"}")));
    const parsed = qs.parse("x=10&y=hello");
    console.log(String(parsed.x) + "," + String(parsed.y));
    console.log(clsx("base", JSON.parse("{\"active\":true,\"off\":false}"), JSON.parse("[\"extra\"]")));
    console.log(slugify("Hello World! Foo & Bar", JSON.parse("{\"lower\":true,\"strict\":true}")));
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
        "a=1&b=2\n10,hello\nbase active extra\nhello-world-foo-and-bar\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}
