/// A real, compiled (native) closure passed as an argument to a dynamic
/// (`JsValue`) method call -- real-world example: zod's `z.number().
/// refine((n: number) => n > 0, {...})`/`.transform(...)`, whose
/// predicate/mapper has no JSON representation at all (`value has type
/// Function([F64], Bool), expected Json`). `coerce_to_declared` (thaw-hir)
/// wraps it in a manual `registerNativeCallback` call, which thaw-llvm's
/// `compile_register_native_callback` turns into a live, retained
/// QuickJS-NG function value: reuses the existing (N-API-oriented but
/// backend-agnostic) `compile_napi_value_callback` adapter, then bridges
/// it into a real callable via a new `thaw_js_register_native_callback`
/// (thaw-quickjs).
///
/// `bundle.js`'s `check` method calls the predicate three times with
/// different arguments to confirm each call round-trips independently
/// (not just a one-shot capture), and the mapper case (`test3`-shaped,
/// folded into this same test) confirms a non-boolean return value
/// marshals correctly too.
///
/// Also confirms a `JsValue` nested inside a *method* call's own argument
/// array works, not just a top-level function call's (`compile_call_
/// dynamic_method`/`compile_call_dynamic_method_handle` previously never
/// set `compiling_quickjs_dynamic_arguments` around their own args
/// marshaling at all, unlike the typed ambient-declaration dispatch path
/// -- confirmed to reproduce the pre-fix "a dynamic (JsValue) value can
/// only be passed as an argument to another QuickJS-backed dynamic call"
/// error via a temporary revert).
#[test]
fn a_native_closure_can_be_passed_as_an_argument_to_a_dynamic_method_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): Holder;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         check: function(pred) { return pred(5) > 0; }, \
         arity: function(pred) { return pred.length; }, \
         map: function(pred) { return pred(1) + \",\" + pred(2) + \",\" + pred(3); } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "callback-kit";
function main(): void {
    const holder: JsValue = makeHolder();
    console.log(holder.check((n: number) => n > 0));
    console.log(holder.check((n: number) => n < 0));
    console.log(holder.arity((a: number, b: number, c: number, d: number) => a + b + c + d));
    console.log(holder.map((n: number) => n * 10));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\nfalse\n4\n10,20,30\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A native closure whose `Json`-typed parameter (real `.d.ts` shape:
/// `(prefix: string, value: any) => any`, e.g. `qs`'s own `filter`
/// option) is returned unchanged, or nested unchanged inside a returned
/// object -- real trigger: `qs.stringify(obj, { filter: (prefix, value)
/// => value })` segfaulted (a real core dump, not a wrong-output bug).
///
/// Root cause, in the one shared adapter every native-callback
/// registration path compiles through
/// (`compile_value_callback_from_closure`, thaw-llvm): every decoded
/// `Json` argument (and any retained `JsValue` parameter handle) was
/// destroyed via `thaw_json_destroy` *immediately* after invoking the
/// closure, before the closure's own return value was ever read to
/// build the JSON result. For a `Json`-typed parameter,
/// `compile_json_value_to_native` passes the decoded pointer straight
/// through with no extra copy, so a closure returning that same
/// parameter (directly, or nested inside another value --
/// `serde_json::Value::clone` is a real recursive clone, so nesting
/// doesn't protect against this either) made the result-building code
/// dereference already-freed memory. Fixed by moving the whole cleanup
/// block (both the `JsValue`-handle release loop and the `Json`-argument
/// destroy loop) to run *after* `result_json` is fully built instead of
/// right after the raw call -- `thaw_json_array_push_json` (used to
/// build `result_json` from the return value) already clones
/// defensively, so nothing needs the original argument pointers again
/// once that has happened, safe whether or not the return value aliased
/// one of them.
#[test]
fn a_native_closure_returning_its_own_json_typed_parameter_does_not_use_after_free() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-native-callback-json-alias-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Options {\n\
         \x20\x20\x20\x20filter?: Array<string | number> | ((prefix: string, value: any) => any) | undefined;\n\
         }\n\
         export declare function invoke(value: any, options?: Options): any;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { invoke: function(value, options) { \
         var filter = options && options.filter; \
         return typeof filter === 'function' ? filter('key', value) : value; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { invoke } from "callback-kit2";
function main(): void {
    console.log(invoke("hello", { filter: (prefix: string, value: any) => value }));
    console.log(JSON.stringify(invoke("world", { filter: (prefix: string, value: any) => ({ wrapped: value }) })));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "exit status: {:?}, stderr: {}",
        result.status.code(),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "hello\n{\"wrapped\":\"world\"}\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A native closure whose body is a ternary unifying an `Undefined`
/// branch with a `Json` branch (`prefix === "b" ? undefined : value`) --
/// real trigger: `qs.stringify(obj, { filter: (prefix, value) =>
/// (cond ? undefined : value) })`, whose `filter` treats an `undefined`
/// return (omit the key) differently from a `null` return (keep the key
/// with an empty value) -- confirmed directly against real Node. Thaw
/// produced the `null` behavior for both, wrong for the `undefined` case.
///
/// Root cause: ternary lowering (`crates/thaw-hir/src/lower/expressions/
/// lowering.rs`, `Expr::Cond`) unifies `(Undefined, Json)` into
/// `HirType::Optional(Json)`, a real native tagged union, not a JSON
/// value yet. When such a closure's inferred return type doesn't match
/// its call site's own declared type (as here: the closure's own
/// annotation-free params/return give it type `Function([Str, Json],
/// Optional(Json))`, while the `.d.ts`'s callback type wants `Json`),
/// the field-typed-as-`Function` branch of `compile_json_object_set_
/// native_with_undefined` (thaw-llvm's `json_bridge/encoding.rs`)
/// registers it as a live native callback via `compile_register_native_
/// callback_from_closure` using the closure's *own* return type (`ret`)
/// directly, without first reconciling it against any wider expected
/// type. That closure's own return-value marshaling, inside `compile_
/// value_callback_from_closure` (`dynamic_host/callbacks.rs`), called
/// `compile_json_array_push_native` -- which hardcodes `preserve_
/// undefined: false` -- to turn its `Optional(Json)` result into JSON,
/// so the tagged union's "absent" case always became a plain JSON
/// `null`, indistinguishable from an explicit `null` return. Same bug
/// in the analogous `Promise<Optional<Json>>` resolved-value path
/// (`compile_native_promise_callback_finisher`).
///
/// Fixed by switching both call sites to `compile_json_array_push_
/// native_with_undefined(..., true)` -- the same `preserve_undefined:
/// true` a standalone value already uses elsewhere (`wrap_native_value_
/// as_json`, thaw-hir) for the identical "this is a value being
/// round-tripped through JSON, not an object field that can
/// legitimately omit itself" reason.
#[test]
fn a_native_closure_returning_undefined_from_one_ternary_branch_is_not_confused_with_null() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-native-callback-ternary-undefined-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit3");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Options {\n\
         \x20\x20\x20\x20filter?: Array<string | number> | ((prefix: string, value: any) => any) | undefined;\n\
         }\n\
         export declare function invoke(value: any, options?: Options): any;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { invoke: function(value, options) { \
         var filter = options && options.filter; \
         var result = typeof filter === 'function' ? filter('key', value) : value; \
         return { \
         typeOfResult: typeof result, \
         isUndefined: result === undefined, \
         isNull: result === null \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { invoke } from "callback-kit3";
function main(): void {
    const outcome: any = invoke("hello", { filter: (prefix: string, value: any) => (prefix === "key" ? undefined : value) });
    console.log(outcome.typeOfResult, outcome.isUndefined, outcome.isNull);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "undefined true false\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A native closure passed as a dynamic-method-call argument (same shape
/// as the test above), whose declared return type is `Promise<T>` --
/// real-world example: drizzle-orm's `sqlite-proxy` driver,
/// `drizzle(callback)`, where `callback` is always
/// `(sql, params, method) => Promise<{rows}>` since it wraps real I/O.
/// Found via drizzle bug-hunting: passing an `async` predicate crashed at
/// *build* time with `"cannot serialize collection element Promise(F64)
/// to JSON"`.
///
/// Root cause: `compile_napi_value_callback` (the callback bridge, reused
/// for both real N-API addons and this QuickJS Fallback path) pushes the
/// callback's raw return value into its JSON result array using the
/// callback's *declared* return type verbatim -- for an `async` callback
/// that's `HirType::Promise(inner)`, but nothing ever stripped the
/// `Promise` wrapper or drove it to resolution first, and
/// `compile_json_array_push_native` has no match arm for
/// `HirType::Promise` at all. Every `is_async` function -- named or an
/// inline async arrow -- always exposes the same real, resolvable
/// `ThawPromise`-pointer ABI regardless of whether its body actually
/// suspends (`discover_frame_async_functions`'s own documented
/// invariant), so the raw return value here is never a raw unwrapped
/// value in disguise -- just an unresolved promise. Fixed by teaching
/// `compile_napi_value_callback` to detect a `Promise`-typed `ret` and
/// drive it to its resolved value via `drive_promise_to_resolved_value`
/// (the value-taking half of the existing `compile_typed_blocking_await`,
/// already used for an ordinary typed `await` expression) before
/// marshaling the *resolved* type, not `Promise<T>`, to JSON. Confirmed
/// to reproduce the exact pre-fix build error via a temporary revert.
#[test]
fn a_promise_returning_native_closure_can_be_passed_as_a_dynamic_method_call_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-async-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("async-callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): Holder;\n\
         interface Holder {\n\
         \x20\x20\x20\x20check(pred: (n: number) => Promise<number>): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         check: function(pred) { \
         Promise.resolve(pred(5)).then(function(result) { console.log('got:' + result); }); \
         } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "async-callback-kit";
function main(): void {
    const holder: JsValue = makeHolder();
    holder.check(async (n: number): Promise<number> => n * 10);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "got:50\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The `async` predicate's `Promise<void>`-resolved counterpart (no
/// meaningful return value at all) -- confirms the null-result path
/// still works correctly once routed through the same Promise-unwrapping
/// branch (real-world example: zod's own `.superRefine((val, ctx) => {
/// ctx.addIssue(...); })`-shaped callbacks, if ever declared `async`).
#[test]
fn a_promise_void_returning_native_closure_argument_produces_undefined() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-async-void-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("async-void-callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): Holder;\n\
         interface Holder {\n\
         \x20\x20\x20\x20check(pred: (n: number) => Promise<void>): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         check: function(pred) { \
         Promise.resolve(pred(5)).then(function(result) { console.log('got:' + result); }); \
         } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "async-void-callback-kit";
function main(): void {
    const holder: JsValue = makeHolder();
    holder.check(async (n: number): Promise<void> => { console.log(n); });
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\ngot:undefined\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A native closure argument whose body `await`s another *held `JsValue`*
/// (not a named `callDynamic*`/method-call form) in **non-tail**
/// position, with no explicit `return` -- exactly the shape of a real
/// hono middleware, `async (c, next) => { ...; await next(); }`. Found
/// while investigating hono's `HandlerInterface` scope boundary: this
/// used to fail to *build at all* with `lambda `__thaw_lambda_N` does
/// not return a value on all paths`.
///
/// Root cause: `await`ing an arbitrary held `JsValue` (invoking `next()`
/// itself lowers to a `callDynamicValue` call, then `await`ing *that*
/// result) was not in `lowering.rs`'s `resolves_at_dynamic_boundary`
/// allowlist -- unlike `callDynamic`/`callDynamicMethod`/etc, which
/// *were* already recognized as synchronous FFI round-trips needing no
/// real suspension. So the closure kept a genuine `HirExpr::Await` node
/// in a non-tail position, forcing the "must really suspend, compile as
/// a raw `Promise`-typed block" path (`functions.rs`) -- but `thaw-llvm`
/// correctly determines this closure never actually needs frame-split
/// codegen (nothing here is a *known* suspend source), so it falls
/// through to plain codegen, which finds no explicit return against the
/// non-`Void` declared type. Fixed by adding `callDynamicValue` (and its
/// `Handle`/`Mixed`/`WithValue` siblings) to the same allowlist as the
/// named dynamic-call forms, since invoking a held `JsValue` is exactly
/// as synchronous as those.
///
/// `next` logging *after* `mw`'s own synchronous "before" log, in the
/// right order, confirms the statement after the non-tail `await`
/// genuinely executes (not just "doesn't crash").
#[test]
fn an_async_closure_argument_can_await_a_held_js_value_in_non_tail_position_with_no_explicit_return(
) {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-async-next-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("middleware-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): Holder;\n\
         interface Holder {\n\
         \x20\x20\x20\x20runMiddleware(mw: (c: JsValue, next: JsValue) => Promise<void>): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         runMiddleware: function(mw) { \
         var next = function() { console.log('next called'); }; \
         mw({}, next); \
         } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "middleware-kit";
function main(): void {
    const holder: JsValue = makeHolder();
    holder.runMiddleware(async (c: JsValue, next: JsValue) => {
        console.log("before");
        await next();
    });
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "before\nnext called\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A native closure passed as an argument to an *ordinary top-level
/// Fallback function call* -- not a method call on a `JsValue` receiver
/// (the shape the three tests above cover, `holder.check(pred)`). Real-
/// world example: drizzle-orm's `sqlite-proxy` driver, `drizzle(callback:
/// (sql, params, method) => Promise<{rows}>)` -- `drizzle` is a plain
/// `declare function`, not a method.
///
/// Found via drizzle bug-hunting: this built successfully but crashed at
/// *runtime* with a JS-side `TypeError: cb is not a function` -- the
/// callback argument was silently never written into the JSON args array
/// at all (`compile_typed_dynamic_call`, thaw-llvm's `dynamic_host.rs`,
/// only marshals a typed function argument for the `napi` backend with a
/// `JsValue` return; every other combination -- including any
/// Fallback/QuickJS call -- has a deliberate `continue` that skips the
/// argument's slot instead of erroring, so the real JS side saw `args[0]
/// === undefined`).
///
/// Root cause, two independent gaps found together:
///
/// 1. `typed_dynamic_declaration` (thaw-cli's `shims.rs`) rendered a
///    `Function`/`CallableFunction`-classified parameter as a real
///    callback type in the generated shim for *both* `napi` and
///    Fallback/QuickJS functions alike -- unlike its sibling
///    `supported_class_method_param` (used for class methods), which
///    already restricts this to the `napi` backend. Fixed by applying the
///    same restriction here: for a non-`napi` function, a
///    `Function`/`CallableFunction`-classified parameter is widened to
///    `Json` (matching how `DtsType::Unsupported` is already handled),
///    which routes it through `coerce_to_declared`(`declared == Json`)'s
///    `registerNativeCallback` bridge instead.
/// 2. Once routed there, this specific test still failed at runtime with
///    "JavaScript value handle registry is empty" -- unlike every other
///    test exercising this bridge, this program's *first-ever* touch of a
///    JsValue handle at all is registering the closure itself (no prior
///    `makeHolder(): JsValue`-style call to have bootstrapped anything
///    first). `retain_value` (thaw-quickjs's `api.rs`, the sole write path
///    into the realm's handle registry) assumed the registry (`__thaw_
///    value_handles`/`__thaw_value_handle_live`) already existed, unlike
///    its near-duplicate sibling `thaw_js_get_global`, which already
///    lazily created it on first use. Fixed by moving that lazy-creation
///    into `retain_value` itself (and simplifying `thaw_js_get_global` to
///    just call it), so *every* path that can be a program's first handle
///    registration self-heals the same way.
///
/// Confirmed to reproduce the pre-fix silent-wrong-output symptom (this
/// doesn't fail to *build* -- it must be asserted via the callback's own
/// observable side effect) via a temporary revert of both fixes together.
#[test]
fn a_native_closure_can_be_passed_as_an_argument_to_an_ordinary_fallback_function_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-plain-callback-arg-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-arg-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function invoke(cb: (x: number) => number): void;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         invoke: function(cb) { console.log('result:' + cb(21)); } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { invoke } from "callback-arg-kit";
function main(): void {
    invoke((x: number): number => x * 2);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "result:42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fallback_functions_can_return_callables_that_return_live_js_values() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callable-js-value-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callable-value-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeLoader(): (id: string) => JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeLoader: function() { return function(id) { return { basename: function(value) { return value.split('/').pop(); } }; }; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeLoader } from "callable-value-kit";
function main(): void {
    const load = makeLoader();
    const module = load("path");
    console.log(module.basename("/tmp/value.txt"));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "value.txt\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// An *optional* callback parameter on an ordinary top-level Fallback
/// function, called with it omitted -- real example: lodash's
/// `filter(collection: string | null | undefined, predicate?:
/// StringIterator<boolean>): string[];` (found while testing the earlier
/// lodash `default`-import fix against real lodash's own full `.d.ts`,
/// via `import * as _ from "lodash"; _.chunk(...)`, which crashed even
/// though `chunk` itself has no callback parameter at all).
///
/// Root cause: an optional callback parameter classifies as `Native(
/// Optional(Function(...)))` -- a *different* `Native` variant from the
/// bare `Native(Function(...))` case the test above covers, so it fell
/// through `typed_dynamic_declaration`'s (thaw-cli's `shims.rs`) widening
/// match arm unwidened. Separately, `typed_dynamic_bare_alias` (the
/// wrapper generated under a function's *bare* name, reached by a plain
/// named import like this test's) had its *own*, independent parameter-
/// type computation with no widening at all, `napi`-aware or not -- so
/// even fixing the first arm alone wasn't enough: calling the bare-name
/// alias with the optional parameter omitted still crashed, since
/// thaw-hir's omitted-trailing-optional-parameter machinery needed to
/// synthesize a value using *this* alias's own (still unwidened)
/// declared type. Both fixed the same way -- widened to `Json` for the
/// non-`napi` backend, matching the existing bare-`Function` case.
#[test]
fn an_optional_callback_parameter_omitted_at_the_call_site_does_not_crash() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-optional-callback-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("filter-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function filter(collection: string, predicate?: (char: string, index: number, s: string) => boolean): string[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         filter: function(collection, predicate) { \
         var out = []; \
         for (var i = 0; i < collection.length; i++) { \
         if (!predicate || predicate(collection[i], i, collection)) out.push(collection[i]); \
         } \
         return out; \
         } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { filter } from "filter-kit";
function main(): void {
    console.log(filter("hello").length);
    console.log(filter("hello", (char: string) => char === "l").length);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\n2\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A dynamic-call argument that's itself a method call chained off a
/// `JsValue` receiver, with no intermediate `const` binding at all --
/// real-world example: zod's `z.string().pipe(z.string().min(3))`,
/// where `.min(3)`'s own result (chained off `z.string()`) is passed
/// straight into `.pipe(...)`'s own argument position. Found via
/// bug-hunting after the native-callback-bridge work: `z.string().
/// pipe(z.string().min(3)).safeParse(...)` silently crashed (exit 1, no
/// output at all -- the same failure shape a missing `JsValue` capture
/// always produces here).
///
/// Root cause: `lower_dynamic_value_method_call`'s own argument-lowering
/// loop called plain `lower_expr` on each argument, with no expected-
/// type hint at all -- so an argument that's itself a further dynamic
/// method call defaulted to the ordinary JSON-decoding snapshot
/// behavior (a content-free `{}`), discarding its real handle, the
/// exact same failure mode `lower_object_lit_field_value` was fixed for
/// at a *different* sink point (an object-literal field, not a method-
/// call argument) earlier in [[project_npm_interop_gaps_2]]. Fixed by
/// giving each argument the same one-shot `Some(&HirType::JsValue)`
/// hint the receiver itself already gets.
///
/// `combine`'s own JS implementation calls a *method* (`.describe()`)
/// on its argument, not just reads a plain data field -- a JSON-
/// stringified snapshot would still carry plain fields like `.tag`
/// correctly (methods just don't serialize), so reading a field alone
/// wouldn't have caught the bug; calling a method the snapshot doesn't
/// have is what actually distinguishes a real live handle from a JSON
/// snapshot here. Confirmed to fail with the pre-fix silent-crash
/// symptom via a temporary revert.
#[test]
fn a_dynamic_call_argument_that_is_itself_a_chained_method_call_stays_live() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-chained-arg-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): Thing;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function build(n) { \
         return { \
         tag: n, \
         combine: function(other) { return build(this.tag + \":\" + other.describe()); }, \
         describe: function() { return this.tag; } \
         }; \
         } \
         module.exports = { makeThing: function() { return build(\"root\"); } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "chain-kit";
function main(): void {
    const a: JsValue = makeThing();
    const b: JsValue = a.combine(makeThing().combine(makeThing()));
    console.log(b.describe());
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "root:root:root\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A `superRefine`-shaped native callback -- real-world example: zod's
/// `z.object({...}).superRefine((val, ctx) => { ctx.addIssue(...); })` --
/// exercising four gaps found and fixed together while bug-hunting after
/// the basic native-callback-bridge work (item 10) landed:
///
/// 1. A callback parameter explicitly typed `JsValue` (`ctx`) -- the
///    bridge previously only marshaled plain JSON-representable
///    parameters into a callback; `compile_json_value_to_native`'s new
///    `HirType::JsValue` case, paired with a `jsvalue_param_mask`
///    thaw-llvm now threads through `thaw_js_register_native_callback`,
///    retains exactly the marked argument positions as a live handle
///    (encoded as the same `{"__thaw_js_handle_id__": N}` marker used
///    for the opposite direction) instead of naively `JSON.stringify`-
///    ing them.
/// 2. A *generic* top-level function's return value used as an inline
///    method-call receiver with no intermediate `const` at all
///    (`object({...}).superRefine(...)`) -- `infer_member_receiver_type`
///    used to exclude *every* generic callee's declared return type
///    (correct when substitution genuinely matters, e.g. `identity<T>(x:
///    T): T`), even when that declared return type was already `JsValue`
///    and thus substitution-independent (an unresolved interface
///    reference like `Schema<T>` can never become JSON-representable
///    no matter what `T` is).
/// 3. A `void`-returning callback (`(val, ctx) => { ctx.addIssue(...); }`,
///    no `return` at all) -- `compile_napi_value_callback`'s adapter used
///    to unconditionally require a real return value.
/// 4. A dynamic call made *from inside* a native callback that was
///    itself invoked *by* an outer dynamic call still on the stack
///    (`ctx.addIssue(...)`, called while the outer `.validate(...)` call
///    that triggered the callback hasn't returned yet) -- `with_context`
///    panicked ("RefCell already borrowed") on the second, reentrant
///    call. Fixed by reusing the `ActiveNapiContext` guard `install_
///    napi_bridge`'s own reentrant call already relies on (despite the
///    "napi" name, a generic "currently active `Ctx`" mechanism, not
///    N-API-specific) via a new `with_active_or_context` (thaw-quickjs).
///
/// `val`'s own numeric fields are read via `Json`/`Number(...)`, not
/// `JsValue` -- deliberately: `val` is plain, JSON-representable data
/// here (matching real zod's own `RefinementCtx` usage, where the
/// *validated value* is ordinary data and only `ctx` itself is a live
/// object with methods), confirming the fix doesn't force every
/// parameter to go through the handle path.
#[test]
fn a_superrefine_shaped_native_callback_with_a_jsvalue_context_parameter_works() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-superrefine-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("super-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function object<T>(shape: T): Schema<T>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeSchema() { \
         return { \
         superRefine: function(check) { \
         return { \
         validate: function(val) { \
         var ctx = { issues: [], addIssue: function(msg) { this.issues.push(msg); } }; \
         check(val, ctx); \
         return ctx.issues; \
         } \
         }; \
         } \
         }; \
         } \
         module.exports = { object: function(shape) { return makeSchema(); } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { object } from "super-kit";
function main(): void {
    const s: JsValue = object({ a: 1, b: 2 }).superRefine((val: Json, ctx: JsValue) => {
        if (Number(val.a) > Number(val.b)) {
            ctx.addIssue("a must be <= b");
        }
    });
    console.log(JSON.stringify(s.validate({ a: 1, b: 2 })));
    console.log(JSON.stringify(s.validate({ a: 3, b: 2 })));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "[]\n[\"a must be <= b\"]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_async_native_callback_can_reenter_quickjs_while_being_polled() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-reentrant-native-promise-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-promise-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): JsValue;\n\
         export declare function run(callback: () => Promise<void>): Promise<void>;\n\
         export declare function runRejected(callback: () => Promise<void>): Promise<string>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeHolder = function() { return { check: function(callback) { return callback('held'); } }; };\n\
         module.exports.run = function(callback) { return new Promise(function(resolve, reject) { setTimeout(function() { var extra = {}; extra.self = extra; Promise.resolve(callback(extra)).then(resolve, reject); }, 0); }); };\n\
         module.exports.runRejected = function(callback) { return Promise.resolve(callback()).then(function() { return 'unexpected'; }, function(error) { return error.message; }); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder, run, runRejected } from "callback-promise-kit";
const holder: JsValue = makeHolder();
async function main(): Promise<void> {
    await run(async (): Promise<void> => {
        await new Promise<void>((resolve): void => resolve());
        console.log(holder.check((value: string): string => value));
    });
    console.log(await runRejected(async (): Promise<void> => { throw "boom"; }));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "held\nboom\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real-world example: hono's `app.get(path, (c) => c.text(...))` --
/// registering a route handler that receives a `JsValue` "context" and
/// returns a live `Response`-shaped object, then reading `.status` off
/// the result of `app.request(path)` (hono's own synchronous testing
/// helper, which invokes the registered handler directly).
///
/// This previously read back `undefined` instead of `200`, with no
/// error at all -- the handler's `return c.text(...)` silently lowered
/// the dynamic method call through the untyped/JSON-decoding dispatch
/// (`callDynamicMethod`, not `callDynamicMethodHandle`), so hono's mock
/// router received a content-free `{}` snapshot instead of the real
/// `Response` handle, and read `.status` off *that*. Two independent
/// gaps, both in thaw-hir, both fixed here:
///
/// 1. `Stmt::Return` (statements/lowering.rs) lowered its argument with
///    plain `lower_expr`, never passing the function's own declared/
///    inferred `ret_type` through as an expected-type hint -- so even an
///    arrow explicitly annotated `(c: JsValue): JsValue => { return c.
///    text(...); }` failed outright ("value has type Json, expected
///    JsValue") until fixed to route through `lower_expr_with_expected_
///    type`.
/// 2. An arrow with *no* return-type annotation at all (the realistic
///    hono handler shape, `(c: JsValue) => c.text(...)` or `(c: JsValue)
///    => { return c.text(...); }`) defaulted its own inferred `ret_type`
///    to `HirType::Dynamic`, starving fix #1's hint of anything useful
///    to propagate. Fixed with a new one-shot `expected_arrow_return_
///    hint`, set by `lower_dynamic_value_method_call`'s own argument-
///    lowering loop (the same place that already hints a chained-call
///    argument as `JsValue`) whenever the argument being lowered is
///    itself an arrow -- letting an unannotated callback passed straight
///    into a dynamic method call default its own return type to
///    `JsValue` instead of `Dynamic`, so a bare tail-position dynamic
///    method call inside it (covering both the braced-body path via fix
///    #1, and the implicit-return expression-body path, which needed its
///    own `lower_expr_with_expected_type` call alongside `Stmt::Return`'s)
///    keeps its real handle.
///
/// Exercises all three handler shapes side by side against independent
/// router instances: an explicit `: JsValue` return annotation, a
/// braced body with no annotation, and an unannotated implicit-return
/// expression body -- confirmed via a temporary revert of both fixes to
/// reproduce the original `undefined` symptom for all three.
#[test]
fn a_dynamic_callback_argument_returning_a_jsvalue_keeps_it_live_even_when_unannotated() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callback-return-jsvalue-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("router-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeRouter(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeRouter() { \
         var handler = null; \
         return { \
         get: function(path, h) { handler = h; }, \
         request: function(path) { \
         var ctx = { text: function(body) { return { status: 200, body: body }; } }; \
         return handler(ctx); \
         } \
         }; \
         } \
         module.exports = { makeRouter: makeRouter };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeRouter } from "router-kit";
function main(): void {
    const appA: JsValue = makeRouter();
    appA.get('/', (c: JsValue): JsValue => { return c.text('one'); });
    console.log(appA.request('/').status);

    const appB: JsValue = makeRouter();
    appB.get('/', (c) => { return c.text('two'); });
    console.log(appB.request('/').status);

    const appC: JsValue = makeRouter();
    appC.get('/', (c) => c.text('three'));
    console.log(appC.request('/').status);

    const appD: JsValue = makeRouter();
    appD.get('/', (c) => c.text(c.missing || 'fallback'));
    console.log(appD.request('/').body);

}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "200\n200\n200\nfallback\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function returns a real, callable closure whose *own*
/// parameter is itself a callback -- real example: ejs's own
/// `ClientFunction`/`AsyncClientFunction` (`compile(...)`'s `client:
/// true` overloads), `(locals?, escape?: EscapeCallback, include?:
/// IncludeCallback, rethrow?: RethrowCallback) => string`. Calling the
/// *returned* closure with a real native closure argument pushes that
/// argument into a JSON arguments array via `compile_json_array_push_
/// native` (the adapter body `compile_js_callback_from_json`, thaw-llvm,
/// builds to call back into JS) -- which had a match arm registering a
/// native callback for a plain `HirType::Function` argument, but not the
/// `HirType::CallableFunction` shape a callback with its *own* optional
/// parameter classifies as, crashing the whole build ("cannot serialize
/// collection element CallableFunction(...) to JSON") regardless of
/// whether the returned closure was ever actually invoked.
#[test]
fn a_returned_callables_own_parameter_can_itself_be_a_real_native_callback() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-returned-callable-callback-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("higher-order-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeApplier(): (value: number, transform?: (n: number) => number) => number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeApplier: function() { \
         return function(value, transform) { \
         return transform ? transform(value) : value; \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeApplier } from "higher-order-kit";
function main(): void {
    const apply = makeApplier();
    console.log(apply(5));
    console.log(apply(5, (n: number): number => n * 10));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\n50\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A rest-parameter callback nested two object levels deep in a Fallback
/// *class instance method*'s argument, compiled through the untyped
/// `callDynamicMethod` JSON path (the class method's argument doesn't
/// score-match its typed declaration once the callback is nested that
/// deep), lost its rest-ness: a bare object-method/arrow `(...args)` is
/// lowered with its trailing rest flattened to one `Array(..)` ABI
/// parameter but inferred as a plain `HirType::Function`, so the
/// native-to-JS adapter built for it passed real JS arguments through by
/// position instead of packing the trailing ones into the array the
/// native side reads -- an uninitialized array, and a segfault the moment
/// the callback indexed it. Real trigger: `new Marked().use({ renderer: {
/// heading(...args) { ... } } })`, marked's own modern custom-renderer API.
///
/// Both spellings a real package uses are covered: a method-shorthand
/// field (`heading`, lowered by `lower_object_method`) and an arrow
/// property (`footnote`, lowered by `lower_arrow`); both are invoked with
/// 0, 1, and 3 real arguments to confirm every trailing argument is
/// collected, byte-identical to real Node.
#[test]
fn a_rest_callback_nested_in_a_class_method_argument_keeps_its_rest_parameter() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-nested-rest-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("marker-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Renderer {\n\
             heading?: (...args: any[]) => string;\n\
             footnote?: (...args: string[]) => string;\n\
         }\n\
         export interface Options { renderer?: Renderer; }\n\
         export declare class Widget {\n\
             use(opts: Options): void;\n\
             invoke0(): string;\n\
             invoke1(): string;\n\
             invoke3(): string;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Widget() { this.renderer = null; }\n\
         Widget.prototype.use = function(opts) { this.renderer = opts.renderer || null; };\n\
         Widget.prototype.invoke0 = function() { return this.renderer.heading(); };\n\
         Widget.prototype.invoke1 = function() { return this.renderer.footnote('x'); };\n\
         Widget.prototype.invoke3 = function() { return this.renderer.heading('a', 'b', 'c') + '|' + this.renderer.footnote('p', 'q', 'r'); };\n\
         module.exports = { Widget: Widget };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Widget } from "marker-kit";
function main(): void {
    const w = new Widget();
    w.use({
        renderer: {
            heading(...args: string[]): string {
                return args.length + ":" + args.join(",");
            },
            footnote: (...args: string[]): string => "fn(" + args.length + "):" + args.join(","),
        },
    });
    console.log(w.invoke0());
    console.log(w.invoke1());
    console.log(w.invoke3());
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "0:\nfn(1):x\n3:a,b,c|fn(3):p,q,r\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic "typed EventEmitter" method -- `on<Event extends keyof
/// Events>(ev: Event, handler: (...args: Events[Event]) => any): this`,
/// the real shape `minipass`'s own `Minipass<RType, WType, Events>` class
/// declares (used by `@isaacs/fs-minipass`'s `ReadStream`/`WriteStream`,
/// among other real packages) -- used to reject an inline callback
/// literal outright: `error: contextual callback accepts at most 0
/// parameter(s), got 1`. A named function reference passed to the
/// identical call already worked; only an inline literal
/// (`function(chunk) {...}`/`(chunk) => ...`) hit this.
///
/// Two independent bugs, both in `contextual_dynamic_type`
/// (`crates/thaw-bridge/src/bridge/dts/classes.rs`), the deliberately
/// lenient re-classification pass a generic class method's own
/// parameters go through after their ordinary (non-generic)
/// classification fails because `Events[Event]` can't resolve `Event` to
/// a literal at `.d.ts`-classification time:
///
/// 1. Its `TsFnOrConstructorType::TsFnType` handling only ever processed
///    `TsFnParam::Ident` parameters, silently dropping a `TsFnParam::
///    Rest` one (`handler`'s only parameter, `...args: Events[Event]`)
///    via `continue` -- `params` stayed empty, producing a zero-argument
///    `HirType::Function` for the whole callback. Fixed by handling
///    `TsFnParam::Rest` and building a `HirType::CallableFunction` with a
///    `JsValue` rest element instead (the same "stay dynamic" fallback
///    `classify_ts_type`'s own non-generic rest-callback handling already
///    uses, just with a `JsValue`, not `Json`, leaf, matching every other
///    leaf in this same function).
/// 2. The caller only reapplied a parameter's substituted (contextual)
///    type when it happened to look like a callback
///    (`hir_type_contains_callback`) -- a *plain* parameter typed
///    directly as the method's own type parameter (`ev: Event`) never
///    got the substitution either, leaving it at its original,
///    unresolvable classification. Fixed by also reapplying it whenever
///    the original classification was `Unsupported`.
///
/// A third, related gap surfaced verifying this end to end once both of
/// the above compiled: a real inline callback with no explicit `return`
/// infers `Void`, but the handler's classified return type (`any`) widens
/// to `JsValue` -- and `return_compatible` (`crates/thaw-hir/src/lower/
/// inference/types.rs`) only ever accepted the reverse direction (a
/// `Void`-*expected* return accepting anything), not a `JsValue`-expected
/// return accepting `Void`. Fixed by extending `return_compatible`'s
/// wildcard to `JsValue` too.
#[test]
fn a_generic_events_map_method_accepts_an_inline_callback_literal() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-events-map-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("event-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface WidgetEvents {\n\
             data: [chunk: string];\n\
             end: [];\n\
         }\n\
         export declare class Widget {\n\
             on<Event extends keyof WidgetEvents>(ev: Event, handler: (...args: WidgetEvents[Event]) => any): this;\n\
             emitData(value: string): void;\n\
             emitEnd(): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Widget() { this._data = null; this._end = null; }\n\
         Widget.prototype.on = function(ev, handler) {\n\
         \x20\x20if (ev === 'data') this._data = handler;\n\
         \x20\x20else if (ev === 'end') this._end = handler;\n\
         \x20\x20return this;\n\
         };\n\
         Widget.prototype.emitData = function(value) { if (this._data) this._data(value); };\n\
         Widget.prototype.emitEnd = function() { if (this._end) this._end(); };\n\
         module.exports = { Widget: Widget };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Widget } from "event-kit";
function main(): void {
    const w = new Widget();
    let total = "";
    w.on("data", function (chunk: any) {
        total += chunk;
    });
    w.on("end", function () {
        console.log("end, total:", total);
    });
    w.emitData("hello ");
    w.emitData("world");
    w.emitEnd();
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "end, total: hello world\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A JavaScript error thrown inside a package's own code, called from a
/// natively-compiled async callback, must keep its *own extra properties*
/// when it propagates back into the package's JS `catch` block (real
/// trigger: koa's `ctx.throw(418, ...)`, whose `http-errors` error carries
/// `status`/`statusCode`/`expose` on its prototype, read by koa's own error
/// handler to answer 418 instead of a generic 500). The exception ABI
/// carries them as a trailing JSON bag.
#[test]
fn a_thrown_js_errors_own_properties_survive_a_native_callback_boundary() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-error-properties-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let thrower = registry.join("boom-kit");
    let handler = registry.join("router-kit");
    std::fs::create_dir_all(&thrower).unwrap();
    std::fs::create_dir_all(&handler).unwrap();
    std::fs::write(
        thrower.join("package.d.ts"),
        "export declare function boom(): void;\n",
    )
    .unwrap();
    std::fs::write(
        thrower.join("bundle.js"),
        "module.exports.boom = function() { var e = new Error('teapot'); e.status = 418; e.expose = true; throw e; };\n",
    )
    .unwrap();
    std::fs::write(
        handler.join("package.d.ts"),
        "export declare function handle(callback: () => Promise<void>): Promise<string>;\n",
    )
    .unwrap();
    std::fs::write(
        handler.join("bundle.js"),
        "module.exports.handle = async function(cb) { try { await cb(); return 'ok'; } catch (e) { return 'err:' + e.status + ':' + e.expose; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { handle } from "router-kit";
import { boom } from "boom-kit";
async function main(): Promise<void> {
    const result = await handle(async (): Promise<void> => {
        boom();
    });
    console.log(result);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "err:418:true\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A live `JsValue` returned by one package call and passed straight into
/// *another* package call's untyped parameter -- with no `const` binding in
/// between (`consume(makeHolder())`) -- must reach the package as the live
/// object, not a content-free JSON snapshot. The argument expression is
/// itself a call, so without a `JsValue` hint it lowered to the JSON-
/// decoding snapshot (`{}`) and the package's own method lookup failed
/// "not a function". Real trigger: rxjs's own
/// `firstValueFrom(of(7).pipe(delay(20)))`, whose `source` parameter is
/// `Observable<T>` -> `Json`.
#[test]
fn a_live_jsvalue_chained_call_argument_reaches_a_package_as_the_object() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-live-chained-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("holder-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Holder { describe(): string; }\n\
         export declare function makeHolder(): Holder;\n\
         export declare function consume(holder: any): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Holder(name) { this.name = name; }\n\
         Holder.prototype.describe = function() { return 'holder:' + this.name; };\n\
         module.exports.makeHolder = function() { return new Holder('gadget'); };\n\
         module.exports.consume = function(holder) {\n\
             return new Promise(function(resolve) {\n\
                 resolve(holder.describe());\n\
             });\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder, consume } from "holder-kit";
async function main(): Promise<void> {
    console.log(await consume(makeHolder()));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "holder:gadget\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A custom property on a caught JavaScript error must be readable
/// (`e.status`), matching real Node -- real trigger: koa's own `onerror`
/// reading an `http-errors` error's `status`/`statusCode`/`expose` to
/// choose the response code. `typeof e` must also report `"object"`.
#[test]
fn a_caught_error_exposes_its_custom_properties_and_object_typeof() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-caught-error-properties-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("boom-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function boom(): void;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.boom = function() {\n\
             var e = new Error('teapot');\n\
             e.status = 418;\n\
             e.statusCode = 418;\n\
             e.expose = true;\n\
             throw e;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { boom } from "boom-kit";
function main(): void {
    try {
        boom();
    } catch (e) {
        console.log(typeof e, (e as any).name, (e as any).message.includes("teapot"), (e as any).status, (e as any).expose);
    }
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "object Error true 418 true\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A native callback parameter annotated `any` that the body **mutates**
/// must be a live `JsValue`, so a package handing the callback its own
/// mutable object sees the mutation. Real trigger: immer's own
/// `produce(value, draft => { draft.x = ... })`, whose `draft` is a Proxy
/// -- with a `Json` snapshot the write landed on a local copy and the
/// returned state was unchanged. A merely-read `any` parameter (a callback
/// that only consumes JSON data) keeps its `Json` shape.
#[test]
fn a_mutating_any_callback_parameter_is_a_live_value() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-mutating-callback-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("mutate-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function withDraft(value: any, mutate: (draft: any) => void): any;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.withDraft = function(value, mutate) {\n\
             var backing = { count: value.count };\n\
             var draft = new Proxy(backing, {\n\
                 get: function(obj, key) { return obj[key]; },\n\
                 set: function(obj, key, val) { obj[key] = val; return true; }\n\
             });\n\
             mutate(draft);\n\
             return backing;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { withDraft } from "mutate-kit";
function main(): void {
    const next: any = withDraft({ count: 1 }, (draft: any): void => {
        draft.count = 42;
    });
    console.log("count", next.count);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "count 42\n");
    let _ = std::fs::remove_dir_all(dir);
}
