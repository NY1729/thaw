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
/// `date-fns@4`'s tree-shakeable functions are generic on their result
/// date: `parseISO<DateType extends Date, ResultDate extends Date =
/// DateType>(argument: string, ...): ResultDate`, and `addDays` /
/// `differenceInDays` similarly. `ResultDate` appears only in the
/// return position -- nothing infers it, and the flattened shim drops
/// its `= DateType` default -- so the call used to fail
/// `cannot infer generic type parameter ResultDate`.
/// `infer_generic_type_tuple` now falls back to a type parameter's
/// `extends` bound (`Date`) when it has neither an inferred value nor a
/// default.
#[test]
fn registry_add_computes_dates_with_real_date_fns_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-date-fns-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "date-fns@4.1.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import { format, addDays, differenceInDays, parseISO, isAfter } from "date-fns";
function main(): void {
    const d = parseISO("2026-01-15T00:00:00Z");
    const later = addDays(d, 10);
    console.log(format(later, "yyyy-MM-dd"));
    console.log(differenceInDays(later, d));
    console.log(isAfter(later, d));
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["date-fns".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).env("TZ", "UTC").output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "2026-01-25\n10\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

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
        r#"import { v4, validate, version, parse, stringify, NIL, MAX } from "uuid";
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

    // `NIL`/`MAX` -- barrel re-exports of another file's own default
    // export, itself a bare literal-typed constant, not a function
    // (`export { default as NIL } from './nil.js'`, `declare const
    // _default: "000...0"; export default _default;`). Regression
    // coverage for a gap where thaw-registry's install-time `.d.ts`
    // flattening never resolved a *value* default re-export, only a
    // function one -- "`uuid` has no export named `NIL`" at build time.
    console.log(NIL);
    console.log(MAX);
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
    assert_eq!(
        lines.next(),
        Some("00000000-0000-0000-0000-000000000000")
    );
    assert_eq!(
        lines.next(),
        Some("ffffffff-ffff-ffff-ffff-ffffffffffff")
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
    // `NO_COLOR` so `isColorSupported` is false regardless of the CI
    // runner's `FORCE_COLOR` / `TERM` -- this test is about the `+`, not
    // the escape codes.
    let result = Command::new(&output)
        .env("NO_COLOR", "1")
        .env_remove("FORCE_COLOR")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // The `+` produced a real string join rather than a build error.
    assert_eq!(
        strip_ansi(&String::from_utf8_lossy(&result.stdout)),
        "ok\nbad\nxy\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Drops CSI escape sequences (`\x1b[ ... m` and friends) so a colour
/// library's output can be asserted against plain text no matter what
/// the runner's `FORCE_COLOR` / TTY detection decides.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            for escaped in chars.by_ref() {
                if escaped.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// `p-limit`'s `LimitFunction` call signature is `<A, R>(fn, ...a):
/// Promise<R>`; the unresolved generic `R` makes `limit(fn)` return
/// bare `Json`, so `Promise.all([limit(fn), limit(fn)])` -- an array of
/// dynamic thenables, not native `Promise`s -- used to fail lowering
/// (`Promise.all element 0 must be a Promise, got Json`). It now routes
/// to QuickJS's own `Promise.all` via `callDynamicMethod`
/// (`docs/design/dynamic-promise-combinators.md`); `allSettled` /
/// `race` / `any` too. `limit.activeCount + " "` (a `JsValue` property
/// read in a string `+`) is covered by the same-session
/// `coerce_primitive_to_string` `JsValue` arm.
#[test]
fn registry_add_awaits_p_limit_promise_all_over_dynamic_thenables_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-p-limit-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "p-limit@6.2.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import pLimit from "p-limit";
async function main(): Promise<void> {
    const limit = pLimit(2);
    const results = await Promise.all([
        limit(async () => 1),
        limit(async () => 2),
        limit(async () => 3),
    ]);
    console.log(results[0] + "," + results[1] + "," + results[2]);
    const settled = await Promise.allSettled([limit(async () => 4), limit(async () => 5)]);
    console.log(settled[0].status + " " + settled[1].value);
    console.log(limit.activeCount + " " + limit.pendingCount);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["p-limit".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "1,2,3\nfulfilled 5\n0 0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `kleur`'s API is `interface Kleur { red: Color; bold: Color; ... }`
/// where `Color` is `interface Color { (x: string | number): string;
/// (): Kleur; }` -- a bare-call-signature (overloaded) interface. Every
/// `Kleur` field is one, so `resolve_interface` (thaw-bridge) collapses
/// `Kleur` to a single opaque `JsValue` rather than a native `Object`
/// of unmaterialisable function fields; `kleur.green("x")` /
/// `kleur.bold().red("y")` then route through the dynamic host.
#[test]
fn registry_add_paints_with_real_kleur_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-kleur-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "kleur@4.1.5").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import kleur from "kleur";
function main(): void {
    console.log(kleur.green("g"));
    console.log(kleur.bold().red("br"));
    console.log(kleur.green("a") + kleur.red("b"));
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["kleur".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output)
        .env("NO_COLOR", "1")
        .env_remove("FORCE_COLOR")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // The chained `bold().red(...)` and the `+` of two results all
    // resolve; ANSI escapes (if the runner forces colour anyway) are
    // stripped -- this test is about the calls, not the codes.
    assert_eq!(
        strip_ansi(&String::from_utf8_lossy(&result.stdout)),
        "g\nbr\nab\n"
    );
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

/// `ky` is the first real package exercised in this suite that reaches
/// `globalThis.fetch` purely through a *member expression*
/// (`globalThis.fetch.bind(globalThis)`) rather than a bare `fetch(...)`
/// call -- `uses_global_fetch`'s AST visitor (thaw-registry's
/// `module_transform.rs`) only ever matched a bound identifier, so ky's
/// own bundle never triggered bundling `node:https`/`node:http` (and
/// thus never got a `globalThis.fetch` at all): `ky.get(url).json()`
/// failed with "fetch is not a function" one level up. Fixed by also
/// matching `globalThis.fetch`/`self.fetch`/`window.fetch`/`global.
/// fetch` member accesses.
///
/// A real `https://` request additionally exposed two more general
/// gaps once fetch itself worked: (1) thaw-cli's `source_uses_tls`
/// (build.rs) is a blind substring scan for `__thaw_tls_` over the
/// generated shim text, but any bundle at or above 1KB gets gzip+
/// base64'd into an opaque blob first (`encode_embedded_script`),
/// hiding the marker inside any real (usually much larger) package --
/// so TLS silently never got linked and every `https://` fetch failed
/// with an opaque "TLS support is not linked"; and (2) rustls
/// surfaces a peer that closes its TCP connection without sending a
/// TLS `close_notify` alert (extremely common in the wild) as
/// `UnexpectedEof` rather than a clean read, even though the response
/// itself was already framed correctly by `Content-Length`/chunked
/// encoding. Both fixed generally: `generate_module_init` (thaw-bridge)
/// now re-emits whichever markers a bundle's *raw* source contains as
/// plain uncompressed comment text before compressing it, and
/// `tls_finish` (thaw-quickjs) now treats `UnexpectedEof` as a clean
/// close. A third gap -- the fetch polyfill's own `Content-Encoding: br`
/// support needs thaw-quickjs's optional Brotli decoder, but the
/// literal word "brotli"/"Brotli" never appears anywhere in the shim or
/// a typical bundle (only the short code `'br'` does), so `source_uses_
/// brotli` never fired for *any* real Brotli response, including a
/// plain `https://example.com/` request through Cloudflare -- fixed by
/// having the fetch polyfill's own source note that it uses Brotli
/// decompression in plain text.
#[test]
fn registry_add_fetches_over_http_and_https_with_real_ky_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-ky-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "ky@2.1.0").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = conn.read(&mut buf).unwrap();
        let body = r#"{"ok":true,"n":42}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        conn.write_all(response.as_bytes()).unwrap();
    });

    std::fs::write(
        &source,
        format!(
            r#"import ky from "ky";
async function main(): Promise<void> {{
    const data: {{ ok: boolean; n: number }} = await ky.get("http://127.0.0.1:{port}/data.json").json();
    console.log(data.ok);
    console.log(data.n);
    // A real Cloudflare-fronted site: exercises the fetch polyfill's
    // real TLS handshake (not the mock server above) and Brotli
    // response decompression together, not just plain local HTTP.
    const text: string = await ky.get("https://example.com/").text();
    console.log(text.length > 0);
    console.log(text.includes("Example Domain"));
}}"#
        ),
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["ky".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    server.join().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\n42\ntrue\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `ms`'s two `.d.ts` overloads -- `(value: number, options?: {long:
/// boolean}): string` and `(value: ms.StringValue): number` -- are
/// type-disjoint only by their shared first argument's runtime `typeof`
/// (both really call the same JS function, which already does this
/// exact dispatch internally), so `union_overload_dispatch_declaration`
/// merges them into one `typeof`-checking dispatcher instead of the
/// ordinary "first overload wins" rule silently picking just one
/// (`shim_generation.rs`). That alone made every *runtime* call work,
/// but the dispatcher's own declared type is necessarily a `string |
/// number` union in and out (nothing about a `typeof` check is visible
/// to the static type system) -- so `const n: number = ms("2 days")`
/// failed to type-check even though the call itself was correct.
///
/// Fixed by having the argument-shape-scoring rewrite pass (already
/// used for e.g. uuid's arity-disjoint overloads) also apply to a
/// union-dispatched name: `union_overload_dispatch_declaration` now
/// returns its own already-declared per-overload symbols as rewrite
/// candidates too, so a call site whose argument statically matches
/// one overload (a number literal, a string literal, or a variable
/// with a known `number`/`string` type) gets rewritten straight to
/// that overload's own precisely-typed symbol -- narrowing the result
/// to a real `number`/`string` instead of the dispatcher's union type.
/// A call whose argument type genuinely isn't known statically still
/// falls through to the original `typeof`-checking dispatcher
/// unchanged.
#[test]
fn registry_add_narrows_real_ms_overloads_by_argument_shape_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-ms-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "ms@2.1.3").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &source,
        r#"import ms from "ms";
function main(): void {
    const n: number = ms("2 days");
    console.log(n);
    const s: string = ms(120000);
    console.log(s);
    const long: string = ms(120000, { long: true });
    console.log(long);
    const value: string = "3 hours";
    const h: number = ms(value);
    console.log(h);
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["ms".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "172800000\n2m\n2 minutes\n10800000\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// axios crashed thaw-bridge outright at build time (not just a
/// runtime error): `interface AxiosStatic extends AxiosInstance`,
/// where `AxiosInstance` itself has bare call signatures (making it
/// opaque per the existing "all-call-signature interface" rule) --
/// `resolve_interface`'s `extends` handling only expected an `Object`,
/// `Dictionary`, or `Unsupported` base and hit its own `unreachable!()`
/// the moment a base resolved to the opaque case instead. Fixed
/// generally in thaw-bridge: an interface extending an opaque base is
/// opaque too, regardless of what fields it adds of its own. See
/// `an_interface_extending_an_opaque_base_stays_opaque_too`
/// (thaw-bridge, network-free) for the isolated shape; this drives the
/// real package end to end -- a plain `axios.get`, `axios.create(...)`
/// building a `baseURL`-scoped instance, and a real network failure
/// surfacing as a caught JS error, not a crash.
#[test]
fn registry_add_fetches_with_real_axios_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-axios-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "axios").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf).unwrap();
            let body = r#"{"ok":true,"n":42}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
        }
    });

    std::fs::write(
        &source,
        format!(
            r#"import axios from "axios";
async function main(): Promise<void> {{
    const response = await axios.get("http://127.0.0.1:{port}/data.json");
    console.log(response.status);
    console.log(response.data.n);
    const instance = axios.create({{ baseURL: "http://127.0.0.1:{port}" }});
    const scoped = await instance.get("/data.json");
    console.log(scoped.data.ok);
    try {{
        await axios.get("http://127.0.0.1:1/nope");
        console.log("unreachable");
    }} catch (error: JsValue) {{
        console.log(typeof error.message);
    }}
}}"#
        ),
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["axios".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    server.join().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "200\n42\ntrue\nstring\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A `this`-dependent method on joi's default-exported, stateful `Joi`
/// namespace object used to crash outright: `Root.string()`/`.object()`/
/// etc are real instance methods, and joi's own internal
/// `internals.generate(this, ...)` asserts `this === root` ("Must be
/// invoked on a Joi instance"). `extract_interface_method_decls` treats
/// every interface method the same way regardless of whether the real
/// implementation depends on `this` (correct for zod's free functions,
/// silently wrong here), extracting it into a detached, receiverless
/// global Fallback function. Fixed generally by `.bind()`ing a copied
/// function value to its own original owner in the two JS-glue binding
/// mechanisms (`crates/thaw-bridge/src/bridge/generation.rs`) --  see
/// `a_stateful_namespace_objects_method_keeps_its_receiver_when_extracted`
/// (network-free, synthetic) for the isolated shape. This drives real
/// joi's full validation flow end to end: a nested schema built from
/// `Joi.object({ ..., name: Joi.string()... })` (also exercises the
/// `Optional`-wrapped dynamic-argument fix, `build_call_with`), a valid
/// and an invalid input.
///
/// Note: `result.error === undefined`-style equality against the
/// literal `undefined` for a dynamic (`Json`/`JsValue`) property whose
/// value genuinely is `undefined` is a separate, pre-existing bug (also
/// reproduces with a plain `JSON.parse(...)`, nothing joi- or
/// this-binding-specific) -- not fixed here, so this test uses
/// `typeof x === "undefined"` instead, which is unaffected.
#[test]
fn registry_add_validates_with_real_joi_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-joi-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "joi").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import Joi from "joi";
function main(): void {
    const schema = Joi.object({
        name: Joi.string().min(2).max(10).required(),
        age: Joi.number().integer().min(0).required(),
    });
    const good = schema.validate({ name: "Alice", age: 30 });
    console.log(typeof good.error === "undefined");
    console.log(JSON.stringify(good.value));
    const bad = schema.validate({ name: "A", age: -1 });
    console.log(typeof bad.error !== "undefined");
}
"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["joi".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\n{\"name\":\"Alice\",\"age\":30}\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The same `this`-dependent-method gap, on handlebars: `Handlebars.
/// registerHelper(...)` reads `this.helpers` and used to fail with
/// "cannot read property 'helpers' of undefined" for exactly the same
/// reason as joi's `Root.string()` above (`Handlebars.compile(...)`
/// itself, by contrast, was already fine -- not `this`-sensitive
/// internally). Drives a registered custom helper through a real
/// `{{#each}}` block plus an unrelated `{{#if}}` block, end to end.
#[test]
fn registry_add_registers_helpers_with_real_handlebars_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-handlebars-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "handlebars").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import Handlebars from "handlebars";
function main(): void {
    Handlebars.registerHelper("upper", (value: string) => value.toUpperCase());
    const template = Handlebars.compile("{{#each items}}{{upper this}} {{/each}}");
    console.log(template({ items: ["a", "b", "c"] }));
    const cond = Handlebars.compile("{{#if flag}}yes{{else}}no{{/if}}");
    console.log(cond({ flag: true }));
    console.log(cond({ flag: false }));
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
        &["handlebars".to_string()],
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
        "A B C \nyes\nno\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// bcryptjs's real entry `.d.ts` (`umd/index.d.ts`) is just `import *
/// as bcrypt from "./types.js"; export = bcrypt; export as namespace
/// bcrypt;` -- every one of its actual functions (`hashSync`,
/// `compareSync`, ...) lives in the sibling `types.d.ts`, which
/// thaw-registry never inlined: it discards every individual `.d.ts`
/// source file except the one flattened `package.d.ts` it writes out,
/// and had no handling for a namespace import (`import * as X from
/// "./y"`) reexported wholesale via `export = X;` (as opposed to
/// `import X = require("./y")`, or `X` being declared directly in the
/// entry file, both already handled). Every call against the package
/// failed to build ("call to unknown function `bcrypt.hashSync`").
/// Fixed generally in thaw-registry's declaration flattening -- see
/// `installed_package_inlines_a_namespace_import_reexported_via_export_
/// assignment` (thaw-registry, network-free) for the isolated shape;
/// this drives the real package end to end, exercising both the sync
/// and async (`Promise`-returning) forms of the same function.
#[test]
fn registry_add_hashes_and_compares_passwords_with_real_bcryptjs_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-bcryptjs-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "bcryptjs").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import bcrypt from "bcryptjs";
async function main(): Promise<void> {
    const hash = bcrypt.hashSync("password123", 10);
    console.log(typeof hash);
    console.log(bcrypt.compareSync("password123", hash));
    console.log(bcrypt.compareSync("wrong", hash));
    const asyncHash = await bcrypt.hash("secret", 10);
    console.log(await bcrypt.compare("secret", asyncHash));
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
        &["bcryptjs".to_string()],
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
        "string\ntrue\nfalse\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// yup's own `object<C = AnyObject, S extends ObjectShape = {}>(spec?:
/// S): ObjectSchema<...>`, called as `yup.object()` with zero
/// arguments, used to fail outright ("cannot infer generic type
/// parameter `S`") -- see
/// `a_generic_fallback_functions_only_optional_parameter_can_be_omitted_
/// entirely` (thaw-cli, network-free) for the isolated shape and root
/// cause. Drives the real package end to end: `yup.object({...})` (the
/// already-working, argument-given form) and `yup.object().shape({...})`
/// (the previously-broken, zero-argument form, chained into `.shape`),
/// plus a real async validation success and failure.
#[test]
fn registry_add_validates_with_real_yup_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-yup-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "yup").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import * as yup from "yup";
async function main(): Promise<void> {
    const schema = yup.object().shape({
        email: yup.string().email().required(),
        age: yup.number().min(0).required(),
    });
    const good = await schema.validate({ email: "a@b.com", age: 30 });
    console.log(JSON.stringify(good));
    try {
        await schema.validate({ email: "not-an-email", age: -1 });
        console.log("unreachable");
    } catch (error: JsValue) {
        console.log(typeof error.message);
    }
}
"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["yup".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "{\"email\":\"a@b.com\",\"age\":30}\nstring\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// ajv's own entry `.d.ts` is `import AjvCore from "./core"; export
/// declare class Ajv extends AjvCore { _addVocabularies(): void; ... }`
/// -- `AjvCore` (`dist/core.d.ts`'s `export default class Ajv { ... }`,
/// essentially ajv's *entire* real API: `compile`, `validate`,
/// `addSchema`, ...) lived in a file thaw-registry otherwise discards,
/// never inlined into the flattened `package.d.ts`. Every call against
/// `new Ajv().compile(...)` (or any other inherited method) failed
/// outright ("call to undeclared function `validate`"), even though
/// `new Ajv()` itself and the 3 methods the entry file adds directly
/// worked fine. Fixed generally in thaw-registry's declaration
/// flattening -- see
/// `installed_package_inlines_a_default_imported_class_used_as_an_extends_base`
/// (thaw-registry, network-free) for the isolated shape; this drives
/// the real package end to end, compiling and running a real JSON
/// Schema against both a valid and an invalid input.
#[test]
fn registry_add_validates_json_schemas_with_real_ajv_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-ajv-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "ajv").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import Ajv from "ajv";
function main(): void {
    const ajv = new Ajv();
    const validate = ajv.compile({
        type: "object",
        properties: {
            name: { type: "string" },
            age: { type: "number" },
        },
        required: ["name", "age"],
    });
    console.log(validate({ name: "Alice", age: 30 }));
    console.log(validate({ name: "Bob" }));
}
"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &["ajv".to_string()]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// validator's real `.d.ts` declares several functions with a genuine
/// union return type (`normalizeEmail(...): string | false`, among
/// others) -- every build against the package failed outright ("typed
/// dynamic return does not support Union([Str, Bool])"), even calling
/// only a plain, non-union function like `isInt`, since every declared
/// Fallback function in the package gets compiled unconditionally. See
/// `a_fallback_functions_union_return_type_decodes_by_runtime_value_
/// shape` (thaw-cli, network-free) for the isolated shape and root
/// cause. Drives real validator end to end: a plain boolean-returning
/// function, plus `normalizeEmail`'s own union return for both a
/// normalizable and (matching real Node's own behavior for this
/// input) a non-normalizable address.
#[test]
fn registry_add_validates_with_real_validator_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-auto-validator-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "validator").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import validator from "validator";
function main(): void {
    console.log(validator.isInt("42"));
    console.log(validator.isEmail("test@example.com"));
    console.log(validator.normalizeEmail("Test@Example.com"));
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
        &["validator".to_string()],
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
        "true\ntrue\ntest@example.com\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// jsonwebtoken end to end: `sign`/`verify` via both the named-import
/// (`import { sign, verify }`) and default-import/namespace
/// (`import jwt from "jsonwebtoken"; jwt.sign(...)`) call forms, an
/// `expiresIn` option, and a wrong-secret verification failure whose
/// caught `JsonWebTokenError`'s `.name` and `.message` must both match
/// real Node exactly. Exercises three real, general bugs found and fixed
/// together while getting this package to build and run correctly:
/// - An overload-dispatch tie-break comparing candidates' *entire*
///   declared parameter vectors instead of just the slots a call site
///   actually provides, which silently misrouted `verify(token, secret)`
///   (2 args) to an overload requiring 3 (`class_methods.rs`).
/// - `node:crypto` missing `KeyObject`/`createSecretKey`/
///   `createPrivateKey`/`createPublicKey` altogether, needed by
///   jsonwebtoken's (and its `jws`/`jwa` dependencies') own real HMAC
///   key-normalization and feature-detection logic (`buffer_crypto.js`,
///   `resolution.rs`).
/// - A registry function returning `JsValue` (matching `verify`'s
///   `JwtPayload | string` union return type) losing a caught custom
///   `Error` subclass's `.name` entirely -- always reporting plain
///   `"Error"` -- because its call convention's exception path
///   (`invoke_raw`) never tagged it, unlike every other call convention
///   (`api.rs`).
#[test]
fn registry_add_signs_and_verifies_real_jsonwebtokens_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-jsonwebtoken-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "jsonwebtoken").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import jwt from "jsonwebtoken";
import { sign, verify } from "jsonwebtoken";
function main(): void {
    const token = jwt.sign({ userId: 42 }, "my-secret", { expiresIn: "1h" });
    const decoded: JsValue = jwt.verify(token, "my-secret");
    console.log(decoded.userId);
    console.log(typeof decoded.iat);
    console.log(typeof decoded.exp);

    const token2 = sign({ role: "admin" }, "another-secret");
    const decoded2: JsValue = verify(token2, "another-secret");
    console.log(decoded2.role);

    try {
        jwt.verify(token, "wrong-secret");
        console.log("unreachable");
    } catch (error: JsValue) {
        console.log(error.message);
        console.log(error.name);
    }
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
        &["jsonwebtoken".to_string()],
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
        "42\nnumber\nnumber\nadmin\ninvalid signature\nJsonWebTokenError\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// luxon end to end: `DateTime.fromISO(...)` (no explicit `locale` --
/// this polyfill always renders English regardless of what locale
/// string luxon itself requests, so real luxon's own default already
/// matches without needing `Settings.defaultLocale` set explicitly;
/// found while writing this test that `Settings.defaultLocale = "en-
/// US"` itself crashes the build entirely -- "needs a monomorphic
/// native implementation" against a synthesized `$new$Settings$arity0`
/// symbol, i.e. a *static property assignment* misclassified as a
/// *constructor* call. A real, separate, general bug, but outside this
/// effort's scope -- noted in `docs/design/intl-polyfill.md` as a
/// follow-up, not chased down here), `.setZone("America/New_York")`
/// across a real DST boundary (July vs. January -- confirms `jiff`'s
/// bundled tzdata, not a hand-rolled DST rule), `.toFormat(...)`,
/// `.toLocaleString(DateTime.DATE_FULL)`, `.toLocaleString(DateTime.
/// DATETIME_FULL)`, and `.diff(...).toHuman()`. Fixed timestamps
/// throughout (never `DateTime.now()`), so this is fully deterministic.
/// See `docs/design/intl-polyfill.md` for the full writeup -- this is
/// the capstone test for that whole effort, exercising the native
/// `jiff`-backed timezone engine, the JS `Intl.DateTimeFormat`/
/// `NumberFormat`/`ListFormat` polyfill, and three separate general
/// compiler/bridge bugs found getting a real, non-constructible-class
/// package's static factory methods (`DateTime.fromISO`) to work at all.
#[test]
fn registry_add_computes_and_formats_real_luxon_datetimes_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-luxon-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "luxon").unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        &source,
        r#"import { DateTime } from "luxon";
function main(): void {
    const summer = DateTime.fromISO("2024-07-04T16:30:45.000Z", { zone: "utc" });
    console.log(summer.toISO());
    const ny = summer.setZone("America/New_York");
    console.log(ny.toISO());
    console.log(ny.offset);
    const winter = DateTime.fromISO("2024-01-04T16:30:45.000Z", { zone: "utc" }).setZone("America/New_York");
    console.log(winter.toISO());
    console.log(winter.offset);
    console.log(summer.toFormat("yyyy-MM-dd HH:mm:ss"));
    console.log(summer.toLocaleString(DateTime.DATE_FULL));
    console.log(ny.toLocaleString(DateTime.DATETIME_FULL));
    const later: JsValue = summer.plus({ days: 3 });
    console.log(later.diff(summer, "days").toHuman());
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
        &["luxon".to_string()],
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
        "2024-07-04T16:30:45.000Z\n\
         2024-07-04T12:30:45.000-04:00\n\
         -240\n\
         2024-01-04T11:30:45.000-05:00\n\
         -300\n\
         2024-07-04 16:30:45\n\
         July 4, 2024\n\
         July 4, 2024 at 12:30 PM EDT\n\
         3 days\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}
