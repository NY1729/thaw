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

/// `infer_member_receiver_type`'s `Expr::Call(inner)` arm, used to
/// classify a chained method call's receiver (`receiver.method(...)`
/// where `receiver` is itself a call expression), only recognized the
/// inner call's callee as dynamic when it was a *declared top-level
/// function* (looked up in `self.signatures`) -- real example: cheerio's
/// `const $ = cheerio.load(html); $("li").each(cb)`. `$` is a local
/// `JsValue`-typed variable (both callable and property-bearing, so it
/// collapses to one opaque handle like any other Fallback value with no
/// single compiled shape), not a declared top-level function, so
/// `self.signatures.get("$")` found nothing and the whole chain fell
/// through to `lower_call`'s final "unsupported member call target"
/// error -- even though calling a bare `JsValue`-typed local directly
/// (`$("li")` with no `.each(...)` chained on it) already worked fine.
/// Fixed by also checking whether the identifier is a locally scoped
/// `JsValue`, in which case calling it is assumed to yield another
/// `JsValue` too -- the same "calling/chaining a `JsValue` produces
/// another `JsValue`" convention already used one arm down for a
/// member-expression callee.
#[test]
fn a_method_can_be_called_on_the_result_of_calling_a_local_dynamic_value() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-call-then-method-on-local-dynamic-value-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("dom-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function load(html: string): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function load(html) {\n\
             return function (selector) {\n\
                 return {\n\
                     each: function (cb) {\n\
                         cb(0, 'a');\n\
                         cb(1, 'b');\n\
                     }\n\
                 };\n\
             };\n\
         }\n\
         module.exports.load = load;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { load } from "dom-kit";
function main(): void {
    const $ = load("<ul><li>a</li><li>b</li></ul>");
    $("li").each((i: number, text: any) => {
        console.log(i, text);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0 a\n1 b\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `Json`-typed dynamic-call argument that itself contains bare
/// `String`/`Number`/`Boolean` values (a mongoose-style schema
/// definition object, `{ name: String, age: Number }`) round-trips
/// correctly through the existing `JsValue`-as-a-JSON-placeholder
/// mechanism (`HirExpr::JsValueAsJson`, `lower/inference/
/// coercions.rs`'s `HirType::Json`-declared branch): each bare
/// constructor value becomes a live-handle placeholder object, JSON-
/// encoded and sent across in one call, and the QuickJS-side reviver
/// splices the real function references back in before the callee
/// ever sees them -- confirmed by checking each field's real
/// `typeof`/`.name` from inside the called function itself, not just
/// that the call didn't crash.
///
/// Known, deliberately out-of-scope companion gap found while writing
/// this test: the *same* object literal coerced to a declared
/// `JsValue` parameter (rather than `Json`) still fails
/// ("value has type Object(...), expected JsValue") -- there is no
/// existing HIR node for "materialize a live JsValue directly from an
/// object literal with mixed native/JsValue fields" the way
/// `JsValueAsJson` already covers the reverse direction. Left for a
/// separate effort: it would need a new HIR node threaded through both
/// codegen backends, not a small extension of this fix.
#[test]
fn json_dynamic_call_argument_carries_real_string_number_boolean_values() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-json-argument-carries-ctor-values-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("schema-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function describeSchema(shape: Json): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function describeSchema(shape) {\n\
             return Object.keys(shape).map(function (key) {\n\
                 var value = shape[key];\n\
                 return key + ':' + typeof value + ':' + (typeof value === 'function' ? value.name : '');\n\
             }).join(',');\n\
         }\n\
         module.exports.describeSchema = describeSchema;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { describeSchema } from "schema-kit";
function main(): void {
    console.log(describeSchema({ name: String, age: Number, active: Boolean }));
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
        "name:function:String,age:function:Number,active:function:Boolean\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `for...of` over a real npm class's `JsValue`-returned method result
/// (real trigger: `lru-cache`'s `LRUCache.keys()`/`.values()`/
/// `.entries()`, each documented `Generator<T, void, unknown>` in the
/// `.d.ts`) used to fail to compile outright: `` `for...of` currently
/// requires a typed array ``. thaw-bridge's own `.d.ts` classifier
/// (unlike thaw-hir's `lower_ts_type`, used for interfaces/plain
/// functions) has no `Generator`/`IterableIterator` case at all, so a
/// real class method declared this way falls back to the generic
/// dynamic escape hatch, `HirType::JsValue` -- and `Stmt::ForOf`
/// (`lower/statements/lowering.rs`) had no case for that at all.
///
/// Fixed by `dynamic_iterator_adapter`, the `JsValue` sibling of the
/// existing `iterator_object_adapter` (used for a *statically*-typed
/// `{next(): {value, done}}` object): it adapts a live `JsValue`
/// iterator into thaw's own generator-producer ABI by calling `.next()`/
/// `.return()`/`.throw()` dynamically (`callDynamicMethod` + `JsonGet`/
/// `JsonAsBool` instead of static `PropAccess`), so it's picked up by
/// the exact same already-existing consumption machinery every other
/// generator shape uses -- including, for free, `.return()` being
/// called automatically on an early `break` (this test collects two
/// values then breaks, confirming the loop stops after exactly two
/// iterations; a real generator's own `finally` block closing correctly
/// is exercised in the real end-to-end `lru-cache` test instead, since
/// observing it here would need a native/JS shared-mutable-state trick
/// this synthetic package's argument marshaling doesn't actually give
/// -- a constructor argument crosses as a JSON snapshot, not a live
/// reference).
#[test]
fn a_for_of_loop_can_iterate_a_jsvalue_returned_generator_and_stops_on_break() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-for-of-jsvalue-generator-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("generator-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Holder {\n\
         \x20\x20\x20\x20constructor();\n\
         \x20\x20\x20\x20items(): Generator<number, void, unknown>;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Holder() {}\n\
         Holder.prototype.items = function*() {\n\
         \x20\x20\x20\x20yield 1;\n\
         \x20\x20\x20\x20yield 2;\n\
         \x20\x20\x20\x20yield 3;\n\
         \x20\x20\x20\x20yield 4;\n\
         };\n\
         module.exports = { Holder: Holder };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Holder } from "generator-kit";
function main(): void {
    const holder = new Holder();
    let collected: string[] = [];
    let count = 0;
    for (const x of holder.items()) {
        count = count + 1;
        collected.push(String(x));
        if (count === 2) break;
    }
    console.log(collected.join(","));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "1,2\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The identical `JsValue`-returned generator (same shape as the test
/// above) confirmed to actually invoke a real generator's own `finally`
/// block -- both on natural exhaustion and on an early `break` -- since
/// `Stmt::ForOf`'s existing close-on-exit machinery calls the producer
/// with `control=1` (`.return()`) in both cases. Uses a method
/// (`wasClosed()`) to read the closed flag back from the *same* live
/// JS-side object afterward, rather than a constructor-argument
/// reference (which would cross as a JSON value snapshot, not a live
/// reference, and so could never observe a JS-side mutation).
#[test]
fn a_for_of_loop_over_a_jsvalue_returned_generator_closes_it_on_early_break_and_on_exhaustion() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-for-of-jsvalue-generator-close-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("generator-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Holder {\n\
         \x20\x20\x20\x20constructor();\n\
         \x20\x20\x20\x20items(): Generator<number, void, unknown>;\n\
         \x20\x20\x20\x20wasClosed(): boolean;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Holder() { this.closed = false; }\n\
         Holder.prototype.items = function*() {\n\
         \x20\x20\x20\x20var self = this;\n\
         \x20\x20\x20\x20try {\n\
         \x20\x20\x20\x20\x20\x20\x20\x20yield 1;\n\
         \x20\x20\x20\x20\x20\x20\x20\x20yield 2;\n\
         \x20\x20\x20\x20} finally {\n\
         \x20\x20\x20\x20\x20\x20\x20\x20self.closed = true;\n\
         \x20\x20\x20\x20}\n\
         };\n\
         Holder.prototype.wasClosed = function() { return this.closed; };\n\
         module.exports = { Holder: Holder };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Holder } from "generator-kit2";
function main(): void {
    const early = new Holder();
    for (const x of early.items()) {
        if (Number(x) === 1) break;
    }
    console.log(early.wasClosed());

    const exhausted = new Holder();
    for (const x of exhausted.items()) {
        void x;
    }
    console.log(exhausted.wasClosed());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\ntrue\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `Array.from` over the identical `JsValue`-returned generator shape --
/// its own separate handling (`("Array", "from")`, `lower/invocations/
/// static_builtins.rs`) doesn't delegate to `for...of`'s machinery at
/// all, and had its own separate `Err` for anything but an array-like
/// `{length}` object, `Array`, `Str`, `Map`, or `Set`. Fixed by eagerly
/// draining a `JsValue` iterator into a plain array via a small
/// self-contained native `while` loop (simpler than routing through
/// `for...of`'s heavier, resumable generator-producer ABI, which this
/// eager one-shot drain doesn't need).
#[test]
fn array_from_can_eagerly_drain_a_jsvalue_returned_generator() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-array-from-jsvalue-generator-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("generator-kit3");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Holder {\n\
         \x20\x20\x20\x20constructor();\n\
         \x20\x20\x20\x20items(): Generator<number, void, unknown>;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Holder() {}\n\
         Holder.prototype.items = function*() {\n\
         \x20\x20\x20\x20yield 10;\n\
         \x20\x20\x20\x20yield 20;\n\
         \x20\x20\x20\x20yield 30;\n\
         };\n\
         module.exports = { Holder: Holder };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Holder } from "generator-kit3";
function main(): void {
    const holder = new Holder();
    const items = Array.from(holder.items());
    console.log(items.length);
    let joined: string[] = [];
    for (const item of items) {
        joined.push(String(item));
    }
    console.log(joined.join(","));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3\n10,20,30\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `Json`-typed value (real trigger: a for-of loop element decoded
/// from *any* dynamic/`JSON.parse`d array, including -- but not limited
/// to -- the new `JsValue`-iterator support above) being `.push()`ed
/// into a *typed* array (`string[]`) used to reject outright: `"array
/// push/unshift value has type Json, expected Str"`, no automatic
/// scalar decode, unlike an ordinary `let`/`const` assignment
/// (`coerce_to_declared` already handles a `Json`-into-scalar slot
/// there). General, found while writing this round's own `lru-cache`
/// comparison script (`keysList.push(k)` where `k` came from `for (const
/// k of cache.keys())`), reproduced independently via a plain
/// `JSON.parse` array with no `JsValue` involved at all -- confirming
/// it's unrelated to that feature, a separate, pre-existing gap.
///
/// Fixed by replacing the strict `expect_type` check `.push()`/
/// `.unshift()` used with an actual `coerce_to_declared` call (`lower/
/// invocations/instance_builtins/array_mutation_methods.rs`) -- which
/// already falls back to that same strict check for anything it can't
/// coerce, so a genuinely incompatible push (a `bool` into a
/// `string[]`, exercised below too) is still rejected exactly as
/// before.
#[test]
fn a_json_value_can_be_pushed_into_a_typed_array_via_automatic_scalar_decode() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-array-push-json-scalar-decode-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let entry = dir.join("main.ts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &entry,
        r#"function main(): void {
    const raw: any = JSON.parse('["a","b","c"]');
    let list: string[] = [];
    for (const item of raw) {
        list.push(item);
    }
    console.log(list.join(","));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "a,b,c\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A genuinely incompatible `.push()` (a `bool` into a `string[]`) is
/// still rejected -- `coerce_to_declared` falls back to the same strict
/// `expect_type` check the fix above replaced whenever it can't coerce,
/// so this must keep failing to compile, not silently push a bogus
/// value.
#[test]
fn pushing_a_genuinely_incompatible_type_into_a_typed_array_still_fails_to_compile() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-array-push-incompatible-type-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let entry = dir.join("main.ts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &entry,
        r#"function main(): void {
    let list: string[] = [];
    list.push(true);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    let error = build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap_err();
    assert!(error.contains("Bool"), "{error}");
    assert!(error.contains("Str"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Destructuring a bare `Json` array value (real trigger: `for (const
/// [k, v] of cache.entries())`, `lru-cache`'s `Generator<[K, V]>`-
/// returning `entries()` -- but reproduced here, and root-caused,
/// independently of that feature via a plain `JSON.parse`d array of
/// pairs) used to fail: `"array pattern requires a fixed-length tuple,
/// got Json"` -- only a statically-known `HirType::Tuple` destructured;
/// a `Json` value (unknown length/shape at compile time) couldn't, even
/// when it's genuinely a fixed-size pair at runtime.
///
/// Fixed by a new `HirType::Json` case in both `lower_binding_pattern`
/// (`destructuring.rs`, a `let`/`const`/for-of declaration) and
/// `lower_assignment_pattern` (`assignments/lowering.rs`, a bare
/// assignment): each position reads out via `JsonIndex`
/// (`thaw_json_index`) and stays `Json`-typed itself, so a nested
/// pattern can keep destructuring further. A rest element has no
/// matching "slice a Json array" primitive, so it's rejected with a
/// clear, narrow error instead of silently doing the wrong thing.
#[test]
fn a_json_array_value_can_be_destructured_by_fixed_position() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-json-array-destructure-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let entry = dir.join("main.ts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &entry,
        r#"function main(): void {
    const raw: any = JSON.parse('[["a",1],["b",2]]');
    let entries: string[] = [];
    for (const [k, v] of raw) {
        entries.push(`${String(k)}:${String(v)}`);
    }
    console.log(entries.join(","));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "a:1,b:2\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `Array.prototype.join` had no `HirType::Json` element case at all
/// (`"array join does not support element type Json"`, `lower/
/// invocations/instance_builtins/array_mutation_methods.rs`) -- only
/// `F64`/`Str`/`Bool`/`Object`. An `Array(Json)` (exactly what a for-of
/// loop over any dynamic/`JsValue` iterator produces, including this
/// round's own new `Array.from(jsValueIterator)`) couldn't be
/// `.join()`'d directly, the literal first error the real `lru-cache`
/// motivating repro (`Array.from(cache.keys()).join(",")`) hit.
///
/// Fixed by building the join directly out of existing HIR nodes
/// instead of adding a new native intrinsic: a `while` loop reading
/// each element via `TypedIndex`, converting it the same way a template
/// literal already does (`coerce_primitive_to_string`, which already
/// handles `Json` via `JsonAsString` -- real JS `String()` semantics,
/// not just "this JSON value happens to already be a string"), and
/// concatenating via the same `__thaw_string_concat` a template
/// literal's own lowering already uses. Exercises both the explicit-
/// separator and default-separator (bare `.join()`) forms, and a
/// non-string element type (numbers) to confirm real `String()`
/// coercion, not a string-only special case.
#[test]
fn array_join_supports_a_generic_json_element_type() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-array-join-json-element-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let entry = dir.join("main.ts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &entry,
        r#"function main(): void {
    const strings: any = JSON.parse('["a","b","c"]');
    let stringArr: any[] = [];
    for (const item of strings) {
        stringArr.push(item);
    }
    console.log(stringArr.join(","));
    console.log(stringArr.join());

    const numbers: any = JSON.parse('[1,2,3]');
    let numArr: any[] = [];
    for (const n of numbers) {
        numArr.push(n);
    }
    console.log(numArr.join("-"));
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
        "a,b,c\na,b,c\n1-2-3\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}
