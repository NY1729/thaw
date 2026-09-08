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
        "true\nfalse\n4\n\"10,20,30\"\n"
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
        "\"root:root:root\"\n"
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"held\"\nboom\n");
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
        "200\n200\n200\n\"fallback\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

