/// `console.log`'s own argument-type lookup (`expr_hir_type`, thaw-llvm)
/// had no entry for `callDynamicMethod` or its sibling manual dynamic-
/// value intrinsics (`getDynamicValue`, `constructDynamicValue`, ...) --
/// thaw-hir's `infer_expr_type` already special-cased these same names,
/// but thaw-llvm's own, separate copy of that classification didn't, so
/// it fell through to `None`. Printing a *bound* result
/// (`const r = thing.describe(); console.log(r);`) worked fine, but
/// passing the call *inline* (no binding) crashed with a segfault --
/// `compile_console_values` mishandling a value it thought had no type.
/// Only reachable at all once `lower_dynamic_value_method_call` (the
/// fix above) started actually generating an inline `callDynamicMethod`
/// call from ordinary `.method()` syntax for the first time.
#[test]
fn console_log_does_not_crash_on_an_inline_dynamic_method_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-inline-dynamic-method-console-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("inline-method-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing() {\n\
             return { describe: function() { return 'no-args-ok'; } };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "inline-method-kit";
function main(): void {
    const thing = makeThing();
    console.log(thing.describe());
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
        "no-args-ok\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `callDynamicMethod` (the fix above routes `.method()` syntax through
/// it) always treats its result as `Json` -- fine for a method that
/// returns plain data (zod's own `.safeParse(...)`), but a method that
/// returns *another* live object (a hypothetical chained schema-builder
/// method returning another schema instance) used to silently produce
/// `{}` instead (a function-only object JSON-stringifies to that)
/// rather than a usable `JsValue`, with no way to ask for the real
/// handle. Fixed by choosing between `callDynamicMethod` and the new
/// `callDynamicMethodHandle` (which retains the result as a handle via
/// `thaw_js_call_method_handle_result` instead of JSON-decoding it)
/// based on this call's own expected-type hint -- the same mechanism an
/// ambiguous `let`/`const` initializer's own type annotation already
/// resolves elsewhere. Exercises both defaults in the same program: an
/// annotated `const wrapped: JsValue = thing.wrap();` gets the real
/// handle (and can have a further method called on *that*, recursing
/// back into the same lowering), while an unannotated call to the same
/// method keeps the old, unchanged behavior.
#[test]
fn a_method_can_return_a_jsvalue_when_the_call_site_asks_for_one() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-returning-method-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit");
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
                 describe: function() { return 'thing:' + name; },\n\
                 wrap: function() {\n\
                     return { describe: function() { return 'wrapped(thing:' + name + ')'; } };\n\
                 }\n\
             };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "chain-kit";
function main(): void {
    const thing = makeThing("gadget");
    const wrapped: JsValue = thing.wrap();
    console.log(wrapped.describe());
    const unannotated = thing.wrap();
    console.log(unannotated);
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
        "wrapped(thing:gadget)\n{}\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// An unannotated method call used as an *object literal field value*
/// (not bound to a `const`, not itself chained further) needs the exact
/// same "keep the real handle" default as a chained-method-call
/// receiver already gets -- real example: zod's own `object({ ...,
/// nickname: string().optional() })`. Before this fix, `.optional()`'s
/// receiver being `JsValue`-typed didn't matter: with no annotation and
/// no further chaining, the field defaulted to the plain JSON-decoding
/// behavior (see `a_method_can_return_a_jsvalue_when_the_call_site_
/// asks_for_one` above), discarding the real handle. That produced an
/// object literal with a field typed plain `Json` holding a content-
/// free snapshot (a schema-builder instance's own state lives behind
/// methods, not serializable fields) -- which a *generic* Fallback
/// function receiving it as part of its inferred type parameter can't
/// specialize for at all (`supports_generic_native_layout` has no case
/// for `Json`, by design: confirmed by direct experiment that loosening
/// it just trades this clean compile error for real zod's own internal
/// validation throwing on the far side of the dynamic call once it
/// doesn't recognize the snapshot as a real schema -- a silent,
/// message-less `exit(1)`).
///
/// Fixed in `lower_object_lit_field_value` (thaw-hir): a field value
/// that's a method call whose receiver is already known to be
/// `JsValue`-typed gets `Some(&HirType::JsValue)` as its own expected-
/// type hint, the same way this session's chained-method-call fix
/// already does for a method call used as *another* method call's own
/// receiver. Verified end-to-end: the field's real handle (with its own
/// methods) survives being embedded in an object literal, passed
/// through a generic Fallback function, and read back out later.
#[test]
fn an_unannotated_method_call_used_as_an_object_literal_field_keeps_its_real_handle() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-object-lit-field-jsvalue-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("shape-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function str(): JsValue;\n\
         export declare function build<T>(shape: T): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.str = function() {\n\
             return {\n\
                 optional: function() {\n\
                     return { toString: function() { return 'optional-str'; } };\n\
                 }\n\
             };\n\
         };\n\
         module.exports.build = function(shape) {\n\
             return {\n\
                 describeNickname: function() {\n\
                     return shape.nickname && typeof shape.nickname.toString === 'function'\n\
                         ? shape.nickname.toString()\n\
                         : 'lost:' + JSON.stringify(shape.nickname);\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { build, str } from "shape-kit";
function main(): void {
    const shape = build({ nickname: str().optional() });
    console.log(shape.describeNickname());
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
        "optional-str\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose inferred type parameter is an
/// *array* of `JsValue` (real example: zod's own `union<T extends
/// readonly core.SomeType[]>(options: T): ZodUnion<T>`, called with
/// `[z.string(), z.number()]`, an array literal of two plain schema-
/// builder function calls) used to fail to specialize outright
/// ("cannot specialize for native layout Array(JsValue)") --
/// `supports_generic_native_layout` (thaw-hir) hardcoded its `Array`
/// case to accept only `F64` elements, unlike `Tuple`/`Object`, which
/// already recursed into every element/field's own type. Confirmed
/// first, directly, that a plain non-generic `JsValue[]` parameter
/// already marshals correctly as an ordinary Fallback argument (unlike
/// the earlier `Object`/`Json`-field gap, which really did mask an
/// unimplemented codegen path) -- so this was genuinely just an
/// unnecessarily narrow check, fixed by making `Array` recurse the same
/// way `Tuple` already does.
#[test]
fn a_generic_call_can_specialize_for_an_array_of_jsvalue_elements() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-array-of-jsvalue-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("union-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function str(): JsValue;\n\
         export declare function num(): JsValue;\n\
         export declare function pick<T>(options: T): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.str = function() {\n\
             return { describe: function() { return 'str'; } };\n\
         };\n\
         module.exports.num = function() {\n\
             return { describe: function() { return 'num'; } };\n\
         };\n\
         module.exports.pick = function(options) {\n\
             return {\n\
                 describeAll: function() {\n\
                     return options.map(function(o) { return o.describe(); }).join(',');\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { str, num, pick } from "union-kit";
function main(): void {
    const picked = pick([str(), num()]);
    console.log(picked.describeAll());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "str,num\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `if`/`while`/`do`/`for`/the ternary's own test/`!x` all used to
/// require an exact `boolean` condition, with no truthiness coercion at
/// all -- real JS lets *any* value be a condition (`truthiness_expr`
/// already backed `Boolean(x)` and `console.assert(x)`, but these six
/// spots never routed through it, each calling `expect_type(&HirType::
/// Bool, ...)` directly instead). Real example: `if (result.success)`
/// against a dynamic-call result's own `Json`-typed `.success` field
/// (`schema.safeParse(...).success`, real zod) -- used to fail outright
/// ("if condition has type Json, expected Bool") even though the exact
/// same value printed or compared fine on its own. Fixed by routing all
/// six through a shared `lower_condition_expr`/direct `truthiness_expr`
/// call instead.
#[test]
fn an_if_condition_accepts_a_json_value_via_truthiness_coercion() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-if-condition-truthiness-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("result-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeResult(ok: boolean): JsValue;\nexport declare function makeValue(ok: boolean): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeResult = function(ok) {\n\
             return { check: function() { return { success: ok }; } };\n\
         };\n\
         module.exports.makeValue = function(ok) { return ok ? { value: 1 } : null; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeResult, makeValue } from "result-kit";
function main(): void {
    console.log(makeValue(false) ? "truthy" : "falsy");
    console.log(makeValue(true) ? "truthy" : "falsy");
    const r = makeResult(true).check();
    if (r.success) {
        console.log("yes");
    } else {
        console.log("no");
    }
    if (!r.success) {
        console.log("negated-yes");
    } else {
        console.log("negated-no");
    }
    const bad = makeResult(false).check();
    console.log(bad.success ? "truthy" : "falsy");
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
        "falsy\ntruthy\nyes\nnegated-no\nfalsy\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A plain property *read* (no call at all) on a `JsValue` receiver used
/// to be entirely unsupported ("unsupported property access `.field` on
/// a value of type JsValue") -- only a *method call* on a `JsValue` was
/// wired up to ordinary syntax (`lower_dynamic_value_method_call`, an
/// earlier session). Real motivating example: real zod's own
/// `ZodError.issues` is deliberately a *non-enumerable* own property (so
/// pretty-printing the error via its own lazy `.message` getter doesn't
/// eagerly serialize every issue) -- meaning an *unannotated* method
/// call's own default JSON-snapshot behavior (a JSON encode can only
/// ever capture enumerable properties) silently loses `.issues`
/// entirely, and there was no other way to reach it at all.
///
/// Fixed by wiring `.property` syntax on a `JsValue` receiver to the
/// existing `getDynamicProperty` intrinsic (`thaw_js_get_property_
/// result`, thaw-quickjs) -- previously only a manual escape hatch,
/// unused by ordinary syntax, the same way `callDynamicMethod` was
/// before *it* got wired up. Reads the property by plain lookup, not by
/// enumeration, so it finds a non-enumerable property correctly.
/// `getDynamicProperty` always hands back a real handle (no JSON-
/// decoding sibling to choose between), so a chained property read
/// (`bad.error.issues.length`) recurses back into the same lowering for
/// free once the outermost receiver is annotated `JsValue` -- confirmed
/// here with only the *outermost* `bad` explicitly annotated, not every
/// intermediate step.
#[test]
fn a_property_can_be_read_on_a_jsvalue_including_a_non_enumerable_one() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-property-read-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("prop-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeThing = function() {\n\
             var inner = { visible: 'v', hidden: 'h' };\n\
             Object.defineProperty(inner, 'hidden', { value: 'h', enumerable: false });\n\
             return { detail: inner, rows: [{ value: 42 }] };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "prop-kit";
function main(): void {
    const thing: JsValue = makeThing();
    console.log(readDynamicValue(thing.detail.visible));
    console.log(readDynamicValue(thing.detail.hidden));
    console.log(readDynamicValue(thing["detail"]["visible"]));
    console.log(readDynamicValue(thing.rows[0].value));
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
        "v\nh\nv\n42\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A method called directly on the result of *another* method call, with
/// no intermediate `const` binding at all -- `z.string().min(2).max(10)
/// .safeParse(...)`, `dayjs(...).add(10, "day").format(...)` -- used to
/// fail to build ("unsupported member call target") past the very first
/// link. Two gaps, both in thaw-hir's `lower/invocations.rs`, fixed
/// together:
///
/// 1. The member-call receiver-type match had no case at all for a
///    receiver that's itself `<expr>.method(...)` (a `Call` whose callee
///    is a `Member`, not a plain `Ident`) -- only a *plain* function call
///    receiver (`dayjs(...).format(...)`, no further chaining) was
///    handled. Fixed by extracting the whole match into a proper
///    recursive method, `infer_member_receiver_type`: when the receiver
///    is itself a method call, it recurses into *that* call's own
///    receiver, and (since a dynamic method's real return type has no
///    declared shape to look up at all) assumes a method invoked on a
///    `JsValue` receiver also yields another `JsValue` -- matching the
///    same "more of the same object" convention real builder-style
///    chains (zod, dayjs) universally follow.
/// 2. Even once the receiver's *type* was known, its actual *value*
///    still came back wrong: `lower_dynamic_value_method_call` lowered
///    its own receiver expression with no expected-type hint, so a
///    receiver that's itself a dynamic method call defaulted to the
///    JSON-decoding behavior (see `a_method_can_return_a_jsvalue_when_
///    the_call_site_asks_for_one` above) instead of a real handle --
///    silently wrong data, not a build error. Fixed by lowering the
///    receiver through `lower_expr_with_expected_type(_, Some(&HirType::
///    JsValue))` instead of the bare `lower_expr`, so every link in the
///    chain unconditionally asks its own receiver for a real handle,
///    recursively.
///
/// Exercises a three-link-deep fully inline chain (no binding anywhere)
/// to confirm both fixes recurse correctly through more than one hop.
#[test]
fn a_method_can_be_chained_directly_onto_another_methods_call_result() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-chained-dynamic-method-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit2");
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
                 append: function(part) {\n\
                     return makeThing(name + '.' + part);\n\
                 },\n\
                 describe: function() { return 'thing:' + name; }\n\
             };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "chain-kit2";
function main(): void {
    console.log(makeThing("root").append("a").append("b").describe());
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
        "thing:root.a.b\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A chain whose intermediate links are themselves "thenable" (an object
/// with a callable `.then`, not a genuine pending Promise) must not have
/// each intermediate result eagerly resolved via `Promise.resolve()` --
/// doing so invokes `.then` prematurely, executing the chain's side
/// effect before later links (`.where(...)`) ever apply. Real-world
/// example: drizzle-orm's `db.select().from(users).where(cond)`, where
/// every query-builder link is a thenable and only the fully-built,
/// awaited chain should execute the query. Reproduces the bug via a
/// minimal synthetic thenable builder instead of the real npm package.
#[test]
fn a_chained_methods_intermediate_thenable_result_is_not_resolved_early() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-chained-thenable-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-thenable-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeDb(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeQuery(steps) {\n\
             return {\n\
                 from: function(part) {\n\
                     console.log('FROM:' + part);\n\
                     return makeQuery(steps.concat(['from:' + part]));\n\
                 },\n\
                 where: function(part) {\n\
                     console.log('WHERE:' + part);\n\
                     return makeQuery(steps.concat(['where:' + part]));\n\
                 },\n\
                 then: function(resolve, reject) {\n\
                     console.log('EXECUTED:' + steps.join(','));\n\
                     return Promise.resolve(steps.join(',')).then(resolve, reject);\n\
                 }\n\
             };\n\
         }\n\
         function makeDb() {\n\
             return {\n\
                 select: function() {\n\
                     console.log('SELECT');\n\
                     return makeQuery(['select']);\n\
                 }\n\
             };\n\
         }\n\
         module.exports.makeDb = makeDb;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeDb } from "chain-thenable-kit";
async function main(): Promise<void> {
    const db: JsValue = makeDb();
    const result: JsValue = await db.select().from("t").where("c");
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
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "SELECT\nFROM:t\nWHERE:c\nEXECUTED:select,from:t,where:c\nselect,from:t,where:c\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The "no import at all" bare-name and package-qualified (`pkg.name(...)`)
/// call syntaxes used to always reach `generate_shim`'s own, always-
/// untyped `(argsArray: Json): Json` fallback -- fine for a single-
/// argument function (which happens to already look like `argsArray`),
/// but broken for any Fallback function taking more than one real
/// argument, since the untyped shape expects its *one* parameter to
/// already be a pre-packed JSON array, not real positional arguments.
/// Real example: `--use semver`, calling bare `semver.major("1.2.3",
/// true)` (or even just bare `major(...)`) with no import written at
/// all. `typed_dynamic_bare_alias` fixes this by forwarding both the
/// bare name and the package-qualified alias to whatever properly-typed
/// declaration `typed_dynamic_declaration` already produced for a plain
/// (non-generic, no `...rest`) Fallback function, instead of leaving
/// them pointed at the untyped fallback. No import anywhere in this
/// program at all -- both calls resolve purely through `--use`.
#[test]
fn bare_and_qualified_calls_with_no_import_reach_a_typed_multi_argument_fallback_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-bare-qualifier-typed-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("verkit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function major(version: string, precise?: boolean): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.major = function(version, precise) {\n\
             var n = parseInt(String(version).split('.')[0], 10);\n\
             return precise ? n + 0.5 : n;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"function main(): void {
    console.log(verkit.major("3.5.1", true));
    console.log(major("3.5.1"));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["verkit".to_string()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3.5\n3\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A two-level member chain (`ns.coerce.number(...)`) through a nested-
/// namespace re-export (`export * as coerce from "...";`) -- real-world
/// example: zod v4's `z.coerce.number()`/`z.iso.datetime()`. Reuses the
/// package.d.ts shape thaw-registry's own flattening produces (see
/// `thaw_bridge::nested_namespace_members`'s doc comment): a synthesized
/// top-level function (`__thaw_ns_coerce_number`, deliberately sharing no
/// name with the package's own top-level `number`, exactly like real
/// zod's `coerce.number` vs. top-level `number`) plus a `declare
/// namespace coerce { export { ... }; }` block recording the real member
/// name it should be reachable under.
///
/// `bundle.js`'s runtime shape is the harder-to-get-right half of this:
/// `coerce.number` is a real *nested* object property (`module.exports =
/// { ..., coerce: { number: fn } }`), not a bare top-level one -- found
/// necessary because a function value reached only through a two-level
/// property chain (`module.exports.coerce.number`), when captured via a
/// *separate*, later `loadScript` call the way an ordinary cross-package
/// collision alias already is, becomes silently uninvokable through the
/// native `callDynamic` FFI boundary (no thrown exception, the whole
/// program just exits 1 with no output at all) despite remaining
/// perfectly callable from JS itself -- confirmed via a minimal, package-
/// agnostic repro. The fix captures a nested-namespace member from
/// *inside* the bundle's own wrapped script instead (right where the
/// ordinary `module.exports` -> `globalThis` copy loop already runs),
/// which this test exercises end to end.
#[test]
fn a_two_level_member_chain_through_a_nested_namespace_reexport_calls_correctly() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-nested-namespace-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit6");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "import * as ns from \"./lib\";\n\
         export * from \"./lib\";\n\
         export { ns, ns as default };\n\
         \n\
         export declare function number(): number;\n\
         export declare function __thaw_ns_coerce_number(): number;\n\
         declare namespace coerce {\n\
         \x20\x20\x20\x20export { __thaw_ns_coerce_number as number };\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { number: function() { return 0; }, coerce: { number: function() { return 42; } } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { ns } from "case-kit6";
function main(): void {
    console.log(ns.number());
    console.log(ns.coerce.number());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}
