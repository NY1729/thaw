/// Constructing a Fallback (pure-JS, QuickJS-NG-dispatched) class
/// instance via `new Class(...)` -- real-world example: hono's `new
/// Hono()`. Two bugs, found and fixed together:
///
/// 1. `generate_napi_class_constructors` (despite the name, also used
///    for a Fallback class's constructor -- see its own `napi`
///    parameter) always emitted `__thaw_typed_napi_...` symbols, which
///    `dynamic_symbol` always decodes as `DynamicBackend::Napi`. For a
///    Fallback class that's the wrong backend outright, and
///    `compile_typed_dynamic_call`'s own construct-a-value branch (LLVM
///    codegen) only existed for `DynamicBackend::Napi` at all -- there
///    was no QuickJS-NG equivalent using `thaw_js_get_global` +
///    `thaw_js_construct_handle_result` (both already existed, wired to
///    nothing).
/// 2. Separately: a package whose *only* export is a class (no
///    top-level Fallback functions at all -- exactly hono's shape) never
///    got its `bundle.js` loaded via `__thaw_module_init` in the first
///    place, since the "does this package need `loadScript`-ing"
///    condition checked only `fallback_names` (top-level functions),
///    never `pkg.classes` -- so even the class's own name was never
///    bound onto `globalThis` at all, regardless of the constructor
///    codegen gap above.
///
/// Also exercises the constructor's own optional-parameter arity
/// dispatch (`new Widget()` vs. `new Widget("hi")`), and that instance
/// methods keep working on a Fallback-constructed instance the same way
/// they already did on one obtained a different way (dayjs's factory
/// function, mime's ready-made export).
#[test]
fn fallback_class_is_constructible_via_new() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-class-new-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("widget-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Widget {\n\
             constructor(name?: string);\n\
             describe(): string;\n\
             $disconnect(): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Widget(name) { this.name = name || 'default'; }\n\
         Widget.prototype.describe = function() { return 'Widget:' + this.name; };\n\
         Widget.prototype.$disconnect = function() { console.log('disconnected'); };\n\
         module.exports.Widget = Widget;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Widget } from "widget-kit";
function main(): void {
    const named = new Widget("hi");
    console.log(named.describe());
    const defaulted = new Widget();
    console.log(defaulted.describe());
    defaulted.$disconnect();
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
        "Widget:hi\nWidget:default\ndisconnected\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A package whose *only* export is a class (no top-level function at
/// all) still gets its `bundle.js` loaded and its exports bound to
/// `globalThis` -- see `fallback_class_is_constructible_via_new`'s doc
/// comment, point 2. This is really the same root cause, but written
/// as its own minimal test (a class exported via a getter-defined
/// property, `Object.defineProperty(module.exports, ..., { get, ...
/// })`, the shape a real esbuild/tsc-bundled package like hono actually
/// uses -- not a plain `module.exports.X = X` assignment) so a future
/// regression here is diagnosable without needing to reason through the
/// constructor-codegen half at all.
#[test]
fn a_class_only_package_still_loads_its_bundle() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-class-only-package-loads-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("getter-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Widget {\n\
             constructor(name?: string);\n\
             describe(): string;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Widget(name) { this.name = name || 'default'; }\n\
         Widget.prototype.describe = function() { return 'Widget:' + this.name; };\n\
         Object.defineProperty(module.exports, 'Widget', { get: function() { return Widget; }, enumerable: true });\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Widget } from "getter-kit";
function main(): void {
    const widget = new Widget("hi");
    console.log(widget.describe());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "Widget:hi\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `JsValue` (the opaque handle a Fallback function returns when its
/// TS type can't be classified as anything JSON-representable, e.g.
/// zod's `z.string()` returning a live `ZodString` schema instance) used
/// to have nowhere to go once it existed: passing one as an argument to
/// *another* dynamic call -- even a bare, top-level one, not nested in
/// anything -- failed at HIR lowering ("value has type JsValue, expected
/// Json"), because there's no JSON encoding of "a live JS object".
/// `compile_dynamic_value_placeholder` (thaw-llvm's `json_bridge.rs`)
/// fixes this by encoding the handle's own permanent id as
/// `{"__thaw_js_handle_id__": N}` instead of trying to serialize it, and
/// the QuickJS-side JSON reviver (see `dates.js`'s doc comment) splices
/// the real value back in the moment that JSON gets parsed on the other
/// end -- the same mechanism already used to round-trip a `Date`.
/// Exercises the value being reused twice (proving the handle stays
/// live/valid across more than one such call, not just one-shot).
#[test]
fn a_js_value_can_be_passed_as_a_bare_argument_to_another_dynamic_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-bare-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("handle-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n\
         export declare function wrapThing(inner: JsValue): JsValue;\n\
         export declare function describe(thing: JsValue): string;\n\
         export declare function makeBox(inner: JsValue): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return { toString: function () { return 'thing:' + name; } };\n\
         }\n\
         function wrapThing(inner) {\n\
             return { toString: function () { return 'wrapped(' + inner.toString() + ')'; } };\n\
         }\n\
         function describe(thing) { return thing.toString(); }\n\
         function makeBox(inner) {\n\
             return { toString: function () { return 'box(' + inner.toString() + ')'; } };\n\
         }\n\
         module.exports.makeThing = makeThing;\n\
         module.exports.wrapThing = wrapThing;\n\
         module.exports.describe = describe;\n\
         module.exports.makeBox = makeBox;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing, wrapThing, describe, makeBox } from "handle-kit";
function main(): void {
    const thing = makeThing("gadget");
    const wrapped = wrapThing(thing);
    console.log(describe(wrapped));
    const boxed = makeBox(thing);
    console.log(describe(boxed));
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
        "wrapped(thing:gadget)\nbox(thing:gadget)\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A bare `undefined` literal passed directly as a dynamic-call argument
/// (real example: zod's own `schema.safeParse(undefined)`, e.g. against
/// a `z.undefined()` schema) used to be a hard compile error ("value has
/// type Undefined, expected Json") -- JSON has no `undefined` at all,
/// only the case a nested *field* can omit itself entirely (already
/// handled elsewhere), which doesn't apply to a standalone value with no
/// field to omit. Fixed in `coerce_to_declared` (thaw-hir) by encoding it
/// as the same `{"$__thaw_napi_undefined$": true}` sentinel a NAPI
/// return value already uses for the identical problem, reviving it back
/// to the real literal on the QuickJS side (`__thaw_json_date_reviver`,
/// the same reviver `Date`/`JsValue` already go through). Distinguishes
/// a real `undefined` from `null` and from any other JSON value to rule
/// out an accidental "map to null" shortcut, which would be wrong: real
/// zod's `ZodUndefined` schema rejects `null`.
#[test]
fn a_bare_undefined_literal_can_be_passed_as_a_dynamic_call_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-arg-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeThing = function() {\n\
             return {\n\
                 check: function(value) {\n\
                     if (value === undefined) return 'real-undefined';\n\
                     if (value === null) return 'null';\n\
                     return 'other:' + JSON.stringify(value);\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "undef-arg-kit";
function main(): void {
    const thing = makeThing();
    console.log(thing.check(undefined));
    console.log(thing.check(null));
    console.log(thing.check(5));
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
        "\"real-undefined\"\n\"null\"\n\"other:5\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// An `Optional`-typed variable (`string | undefined`) that actually
/// holds `undefined` at runtime, passed as a dynamic-call argument
/// (real example: `schema.safeParse(value)` against real zod's own
/// `z.undefined()`, where `value: string | undefined` happens to be
/// `undefined`) needs the exact same real-`undefined` round-trip the
/// bare-literal case above gets. Exercised through `wrap_native_value_
/// as_json`'s own temporary-object round trip (`JsonSet` then `JsonGet`)
/// -- unlike the bare-literal case, this one goes through the tagged
/// `Optional`/`Nullable`/`Nullish` encoding (`compile_json_object_set_
/// tagged`), which has its own long-standing `preserve_undefined` flag
/// for exactly "should an absent value be written as a real `undefined`
/// sentinel or simply omitted" -- omission is correct for a genuine
/// object-literal field (matching `JSON.stringify`'s own behavior), but
/// wrong here: there's no real field to omit, just a temporary one used
/// to round-trip a *standalone* value back out, and omitting it made
/// `thaw_json_get`'s own missing-key fallback report plain JSON `null`
/// instead -- silently turning a real `undefined` argument into `null`,
/// which real zod's `ZodUndefined` schema correctly rejects (`false`
/// where `true` was expected). Fixed by threading a `bool` through
/// `HirExpr::JsonSet` so `wrap_native_value_as_json`'s own use of it can
/// ask for `true` (preserve) while an ordinary object literal's own
/// field-by-field construction keeps the old `false` (omit) behavior.
#[test]
fn an_optional_variable_holding_undefined_can_be_passed_as_a_dynamic_call_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-optional-undefined-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("opt-undef-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeThing = function() {\n\
             return {\n\
                 check: function(value) {\n\
                     if (value === undefined) return 'real-undefined';\n\
                     if (value === null) return 'null';\n\
                     return 'other:' + JSON.stringify(value);\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "opt-undef-kit";
function main(): void {
    const thing = makeThing();
    const absent: string | undefined = undefined;
    console.log(thing.check(absent));
    const present: string | undefined = "hi";
    console.log(thing.check(present));
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
        "\"real-undefined\"\n\"other:\\\"hi\\\"\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Same underlying gap as
/// `a_js_value_can_be_passed_as_a_bare_argument_to_another_dynamic_call`,
/// but for a `JsValue` nested inside an object-literal field and inside
/// an array-literal element -- both go through the *same*
/// `compile_dynamic_value_placeholder` call, just reached via
/// `compile_json_object_set_native_with_undefined`'s and
/// `compile_json_array_push_native_with_undefined`'s own `HirType::JsValue`
/// arms rather than the bare-argument catch-all, and the QuickJS-side
/// reviver splices each one back in regardless of depth (it runs
/// bottom-up over the whole parsed value, exactly like it already does
/// for a nested `Date`). Real-world shape: zod's own `z.object({ name:
/// z.string() })`, a fixed-shape object literal with one field itself a
/// live schema value.
#[test]
fn a_js_value_can_be_nested_inside_an_object_or_array_literal_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-nested-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("handle-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n\
         export declare function group(shape: { name: JsValue; label: string }): string;\n\
         export declare function listGroup(items: JsValue[]): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return { toString: function () { return 'thing:' + name; } };\n\
         }\n\
         function group(shape) {\n\
             return shape.label + '=' + shape.name.toString();\n\
         }\n\
         function listGroup(items) {\n\
             return items.map(function (item) { return item.toString(); }).join(',');\n\
         }\n\
         module.exports.makeThing = makeThing;\n\
         module.exports.group = group;\n\
         module.exports.listGroup = listGroup;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing, group, listGroup } from "handle-kit2";
function main(): void {
    const a = makeThing("alpha");
    console.log(group({ name: a, label: "x" }));
    const b = makeThing("beta");
    console.log(listGroup([a, b]));
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
        "x=thing:alpha\nthing:alpha,thing:beta\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose type parameter is constrained to
/// some other *named* type (`T extends core.SomeType`) -- an extremely
/// common TS generics idiom, and zod's own real shape for e.g.
/// `optional<T extends core.SomeType>(innerType: T): ZodOptional<T>` --
/// used to make `typed_dynamic_declaration` (thaw-cli's `shims.rs`)
/// give up on the *whole* declaration, since `is_reparseable_ts_type`'s
/// constraint allowlist is just a handful of primitive keywords
/// (`string`, `number`, `Date`, ...), falling all the way back to the
/// bare untyped `(argsArray: Json): Json` shim -- silently wrong for an
/// ordinary single-argument call like this one, the same failure mode
/// as an unresolvable parameter or return type already had its own
/// fallback for. Fixed by dropping an unparseable constraint (rendering
/// the type parameter bare) instead of aborting the declaration, and by
/// teaching `supports_generic_native_layout` (thaw-hir) that `JsValue`
/// specializes a generic type parameter exactly like any other
/// fixed-size scalar -- needed here because `T` infers as `JsValue` from
/// `makeThing`'s own `JsValue`-returning result.
#[test]
fn a_generic_function_constrained_to_a_named_type_still_gets_a_typed_declaration() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-named-constraint-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("generic-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n\
         export declare function wrapGeneric<T extends core.SomeType>(inner: T): JsValue;\n\
         export declare function describe(thing: JsValue): string;\n\
         declare namespace core { interface SomeType {} }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return { toString: function () { return 'thing:' + name; } };\n\
         }\n\
         function wrapGeneric(inner) {\n\
             return { toString: function () { return 'wrapped(' + inner.toString() + ')'; } };\n\
         }\n\
         function describe(thing) { return thing.toString(); }\n\
         module.exports.makeThing = makeThing;\n\
         module.exports.wrapGeneric = wrapGeneric;\n\
         module.exports.describe = describe;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing, wrapGeneric, describe } from "generic-kit";
function main(): void {
    const thing = makeThing("gadget");
    const wrapped = wrapGeneric(thing);
    console.log(describe(wrapped));
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
        "wrapped(thing:gadget)\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// End-to-end regression for the bug that turned out to be blocking real
/// zod, not the two generic-declaration gaps fixed just above: zod
/// exports a schema-builder function literally named `undefined`
/// (`z.undefined()`), and `wrap_as_commonjs_module`'s `module.exports`
/// -> `globalThis` copy loop (thaw-bridge) used to abort entirely the
/// moment one key's assignment threw (`globalThis.undefined` is
/// non-writable) -- silently dropping every export enumerated after it
/// too, `after` included, even though it has nothing to do with
/// `undefined`. See
/// `a_key_that_cannot_bind_to_globalthis_does_not_block_later_exports`
/// (thaw-bridge's own tests) for the unit-level regression coverage;
/// this is the same bug reproduced with a real `--use`d package and a
/// real import, the shape that actually surfaced it. Both functions
/// return `JsValue` (an opaque, un-JSON-representable handle, matching
/// real zod's own schema-builder return shape) rather than a plain
/// string -- a `Fallback` function simple enough to synthesize a
/// numeric/string result gets JIT-compiled directly by thaw itself
/// (`jit_export`) and so never actually loads `bundle.js` into QuickJS
/// at all, which would silently skip the very code path this test
/// means to exercise.
#[test]
fn an_export_literally_named_undefined_does_not_block_later_exports() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-export-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function undefined(): JsValue;\n\
         export declare function after(name: string): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "exports.undefined = function() { return { toString: function() { return 'u'; } }; };\n\
         exports.after = function(name) { return { toString: function() { return '[' + name + ']'; } }; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { after } from "undef-kit";
function main(): void {
    const r = after("hi");
    console.log(r);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "[hi]\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A *second*, more insidious bug from the same root cause as the test
/// above -- found while confirming real zod's own `z.undefined()`
/// works (it still doesn't; see [[project_npm_interop_gaps_2]] for why
/// that part is architectural, not fixed here). `typed_dynamic_bare_
/// alias` (the bare/qualified-call-syntax fix) generates a plain,
/// bare-named top-level declaration for *every* Fallback function
/// unconditionally, regardless of whether it's actually imported --
/// including a function literally named `undefined`. thaw-hir's own
/// `Expr::Ident` lowering treats a bare reference to `undefined`
/// specially (the JS literal) *unless* `self.signatures` -- a flat,
/// whole-program table -- already has a real entry under that exact
/// name, in which case it's treated as a reference to *that* function
/// value instead. Since `self.signatures` has no per-call-site
/// disambiguation, declaring a bare `function undefined(...)` *anywhere*
/// silently broke `!= undefined`/`=== undefined` comparisons
/// *everywhere else in the compiled program* -- including inside
/// thaw's own generated arity-dispatch wrapper's `param != undefined`
/// optional-parameter guard for a *different*, otherwise-uninvolved
/// function, which crashed with "numeric conversion is not defined for
/// native type Optional(...)" the moment it tried comparing a real
/// argument against what it thought was the `undefined` literal but was
/// actually a function value.
///
/// Fixed by skipping the bare (non-qualified) form entirely for a name
/// thaw-hir gives this special global meaning to (`undefined`, `NaN`,
/// `Infinity` -- see `shadows_a_thaw_literal_identifier`'s own doc
/// comment) in both `typed_dynamic_bare_alias`'s caller and the older,
/// untyped `generate_shim`/`generate_native_addon_shim` fallback (which
/// has the exact same risk on its own, independent of the newer bare-
/// alias machinery). The package-qualified alias is unaffected --
/// unrelated to this test, since it can never collide with a bare
/// literal reference.
///
/// Exercises exactly the failure shape: a package exports something
/// under the literal name `undefined` (with an optional parameter, so
/// its own wrapper needs a `!= undefined` guard) *and* a second,
/// unrelated function whose own optional-parameter guard needs the
/// *real* `undefined` literal to keep working, *and* the user's own
/// code compares an unrelated value against `undefined` directly --
/// all three used to be silently corrupted by the mere presence of the
/// `undefined`-named export, even without ever calling it.
#[test]
fn an_export_literally_named_undefined_does_not_corrupt_undefined_comparisons_elsewhere() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-export-comparisons-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Params { message?: string; }\n\
         export declare function undefined(params?: string | Params): string;\n\
         export declare function safe(value?: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.undefined = function(params) { return 'ok:' + JSON.stringify(params ?? null); };\n\
         module.exports.safe = function(value) { return value === undefined ? -1 : value * 2; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as nk from "undef-kit2";
function main(): void {
    const x: number | undefined = undefined;
    console.log(x === undefined);
    console.log(nk.safe());
    console.log(nk.safe(5));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\n-1\n10\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function literally named `undefined` (real example: zod's
/// own `z.undefined()`) used to be uncallable outright, in addition to
/// the comparison-corruption bug the test above fixes: the runtime
/// dynamic-dispatch mechanism binds every export onto `globalThis` by
/// its own JS-side key first (`generate_module_init`'s own alias-
/// capture snippet used to read `globalThis.{bare_name}` directly), and
/// `globalThis.undefined` can never be reassigned in *any* JS engine --
/// a real ECMAScript restriction, not a thaw bug -- so the qualified key
/// the actual dynamic call looks up by never got bound to anything at
/// all for this one name, no matter what the TS-side declaration looked
/// like (`callDynamic("pkg::undefined", ...)`'s own runtime lookup found
/// nothing).
///
/// Fixed by reading `globalThis.module.exports.{bare_name}` first
/// instead -- `globalThis.module` still holds *this* package's own
/// fresh `{ exports: {} }` at the exact point this capture runs (nothing
/// else has run in between), so `module.exports.undefined` is a
/// perfectly ordinary object-property lookup, immune to the
/// `globalThis.undefined` restriction, regardless of what `bare_name`
/// is. Falls back to the old `globalThis.{bare_name}` read only when
/// that property lookup finds nothing (the one shape it doesn't cover:
/// a CommonJS package whose whole `module.exports`, not a property of
/// it, is the single exported function).
#[test]
fn an_export_literally_named_undefined_is_actually_callable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-export-callable-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-kit3");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Params { message?: string; }\n\
         export declare function undefined(params?: string | Params): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.undefined = function(params) { return 'ok:' + JSON.stringify(params ?? null); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as nk from "undef-kit3";
function main(): void {
    console.log(nk.undefined());
    console.log(nk.undefined("hello"));
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
        "ok:null\nok:\"hello\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A `JsValue` receiver (a Fallback return value with no compiled class
/// behind it, e.g. zod's `z.object(...)` returning a live `ZodObject`)
/// couldn't have any of its own methods called at all --
/// `schema.safeParse(data)` failed to even build ("call to unknown
/// function `schema.safeParse`"), since thaw-hir's member-call lowering
/// only recognized a receiver typed as a known native *class*, erroring
/// for anything else instead of falling through to `callDynamicMethod`
/// (an existing low-level intrinsic for calling a named method on a
/// retained value by handle -- previously only a manual escape hatch,
/// never actually wired up to ordinary `.method(...)` syntax).
/// `lower_dynamic_value_method_call` (thaw-hir) fixes this, reusing the
/// same `Json`-laundering `coerce_to_declared` already does everywhere
/// else to build the method's argument array. Exercises both a bound
/// and a fully inline (unbound, chained straight into a property
/// access) call -- the inline shape needs its own fix too, see
/// `console_log_does_not_crash_on_an_inline_dynamic_method_call` below.
#[test]
fn a_method_can_be_called_on_a_jsvalue_returned_by_a_fallback_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-method-call-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("method-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return {\n\
                 describe: function() { return { text: 'thing:' + name }; },\n\
                 rename: function(next) { return { text: 'renamed:' + next }; }\n\
             };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "method-kit";
function main(): void {
    const thing = makeThing("gadget");
    const described = thing.describe();
    console.log(described.text);
    console.log(thing.rename("widget").text);
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
        "\"thing:gadget\"\n\"renamed:widget\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

