/// A Fallback (QuickJS-NG, no native addon) package whose factory
/// function returns an instance of one of its own classes -- real-world
/// example: dayjs's `declare function dayjs(...): dayjs.Dayjs`, `Dayjs`
/// declared inside `declare namespace dayjs { class Dayjs {...} }`
/// rather than at the top level. Item 4 of
/// docs/design/npm-interop-gaps-2026-09.md: `pkg.classes` used to be
/// bridged only for a native-addon package (`generate_native_addon_shim`
/// gated on `pkg.native_addon.is_some()`); a Fallback class's instance
/// stayed a permanently opaque, method-less `JsValue`, and a factory
/// function's return value in particular had no linkage back to which
/// class it even was (a namespace-qualified return type like
/// `dayjs.Dayjs` classifies `Unsupported` and loses the name).
/// Exercises the whole chain: `thaw_bridge::function_return_named_types`
/// linking the factory to its class, `parse_dts_classes` now descending
/// into `declare namespace` blocks to find the class at all, thaw-cli's
/// shims.rs generating a QuickJS-NG-backed (`__thaw_typed_js_`) instance
/// method declaration via the same generator a native addon's class
/// already used, `compile_typed_napi_method` (thaw-llvm) now dispatching
/// that declaration's backend to `thaw_js_call_method_result` instead of
/// only ever `thaw_napi_call_method_typed_result`, and class_methods.rs
/// tracking the factory call's result as a class instance the same way
/// `new ClassName(...)` already is.
#[test]
fn fallback_factory_function_result_supports_instance_method_calls() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-factory-class-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clock-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeClock(hour: number): clock.Clock;\n\
         declare namespace clock {\n\
             class Clock {\n\
                 format(): string;\n\
                 hourAt(offset: number): number;\n\
             }\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Clock(hour) { this.hour = hour; }\n\
         Clock.prototype.format = function() { return 'H:' + this.hour; };\n\
         Clock.prototype.hourAt = function(offset) { return this.hour + offset; };\n\
         module.exports.makeClock = function(hour) { return new Clock(hour); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeClock } from "clock-kit";
function main(): void {
    const c = makeClock(3);
    console.log(c.format());
    console.log(c.hourAt(5));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "H:3\n8\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Same shape as `fallback_factory_function_result_supports_instance_
/// method_calls`, but the method is called *chained directly* off the
/// factory call's own return value (`makeClock(3).format()`), with no
/// `const` binding in between -- real example: dayjs's own `dayjs(
/// "2024-01-15").format("YYYY-MM-DD")`. Used to fail to build
/// ("unsupported member call target"): the receiver-type lookup in
/// thaw-hir's member-call lowering only handled a plain `Ident` (bound
/// via `const`, recovering its type from `self.scope`), `new`, `this`,
/// and a few other forms -- a bare `Expr::Call` receiver had no arm at
/// all and fell through to `None`, so the call's *known* factory return
/// type (`clock.Clock`) was simply never consulted. Fixed by looking up
/// the callee's own declared (non-generic) return type directly from
/// `self.signatures` for exactly this shape; the receiver expression
/// itself is still only lowered and evaluated once, when it's spliced
/// into the class-method call the same way a bound receiver's `Ident`
/// already was.
#[test]
fn a_method_can_be_chained_directly_onto_a_fallback_factory_calls_return_value() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-factory-chained-call-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clock-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeClock(hour: number): clock.Clock;\n\
         declare namespace clock {\n\
             class Clock {\n\
                 format(): string;\n\
             }\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Clock(hour) { this.hour = hour; }\n\
         Clock.prototype.format = function() { return 'H:' + this.hour; };\n\
         module.exports.makeClock = function(hour) { return new Clock(hour); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeClock } from "clock-kit2";
function main(): void {
    console.log(makeClock(3).format());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "H:3\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function with two overloads whose *type-disjoint* first
/// parameter picks between genuinely different calling conventions --
/// real-world example: `ms`'s own `ms(value: number, options?: { long:
/// boolean }): string` and `ms(value: ms.StringValue): number` (the
/// same function either formats a millisecond count as a string or
/// parses a duration string into a millisecond count). `typed_dynamic_
/// declaration`'s "first successful overload wins" rule (needed to
/// avoid emitting two conflicting ambient declarations under the same
/// name) used to pick only the first-declared overload, silently
/// breaking every call shaped like the other -- `ms("2 days")` failed
/// with a type error demanding a `number`. Also exercises two
/// prerequisites this needed: a namespace-qualified type reference
/// (`ms.StringValue`) resolving through `declare namespace ms { ... }`
/// at all (`extract_type_alias_decls`), and a template literal type
/// resolving to `Str` (ms's own `StringValue` is a union of them).
#[test]
fn fallback_function_with_type_disjoint_overloads_dispatches_by_typeof() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-dispatch-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("duration-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function duration(value: number, options?: { long: boolean }): string;\n\
         declare function duration(value: duration.StringValue): number;\n\
         declare namespace duration {\n\
             type StringValue = `${number}` | `${number} days`;\n\
         }\n\
         export = duration;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = function(value, options) {\n\
             if (typeof value === 'number') {\n\
                 return options && options.long ? value + ' milliseconds' : value + 'ms';\n\
             }\n\
             var days = parseInt(value, 10);\n\
             return days * 86400000;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import duration from "duration-kit";
function main(): void {
    console.log(duration("2 days"));
    console.log(duration(60000));
    console.log(duration(60000, { long: true }));
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
        "172800000\n60000ms\n60000 milliseconds\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose return type is declared literally
/// `any` -- real-world example: lodash's `cloneDeepWith<TValue>(...):
/// any`. The generic branch's "a return that doesn't mention any type
/// parameter is just a concrete type" rule (see
/// `fallback_function_with_a_generic_date_argument_and_return_builds_
/// and_runs`) used to splice that `any` text straight into the
/// generated ambient declaration's own return position -- reparseable
/// for a type parameter's own `extends` *constraint* (`is_reparseable_
/// ts_type`, since thaw-hir keeps a constraint as raw, unlowered
/// syntax), but not for an ordinary *value* type position, which does
/// get lowered through thaw-hir's `lower_ts_type` and only supports
/// `number`/`string`/`boolean`/`void` there -- so the whole generated
/// shim failed to compile ("unsupported type keyword TsAnyKeyword")
/// the moment any Fallback package had a generic function shaped this
/// way, even one the user's own program never calls.
#[test]
fn generic_fallback_function_with_a_literal_any_return_type_builds() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-generic-any-return-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clone-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function cloneDeepWith<T>(value: T, customizer: unknown): any;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.cloneDeepWith = function(value) { return value; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { cloneDeepWith } from "clone-kit";
function main(): void {
    console.log("built");
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "built\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `Str`/`F64`-type-disjoint overload set whose two overloads name
/// their (structurally corresponding) first parameter *differently* --
/// unlike this session's own `ms` regression test, whose two overloads
/// happen to both call theirs `value`, masking a real bug:
/// `union_overload_dispatch_declaration`'s generated dispatcher
/// referenced each branch's *own* parameter name when forwarding the
/// call (`secondary_overload(name)`) instead of the dispatcher's own
/// shared variable for that slot (`primary`'s name, since the
/// dispatcher's outer signature is named after whichever overload
/// carries extra parameters) -- an undefined-reference bug that would
/// otherwise make the "secondary" (non-primary) branch always forward
/// `undefined` instead of the real argument.
#[test]
fn fallback_function_with_differently_named_overload_parameters_forwards_the_shared_variable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-param-names-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("pick-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function pick(word: string): number;\n\
         export declare function pick(count: number, label?: string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.pick = function(a, b) {\n\
             return typeof a === 'string' ? a.length : a + (b ? b.length : 0);\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { pick } from "pick-kit";
function main(): void {
    console.log(pick("hello"));
    console.log(pick(10, "ab"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\n12\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `Bool`/`F64`-discriminated overload set (real example: lodash's
/// `random(floating?: boolean): number` / `random(max: number,
/// floating?: boolean): number`) whose generated dispatcher's shared
/// first-parameter variable is named after the primary overload's own
/// parameter (`max`) -- which, in a real package like lodash, collides
/// with an unrelated top-level function *also* named `max` (`_.max`)
/// declared elsewhere in the same `.d.ts`. `typeof`-lowering
/// (`UnaryOp::TypeOf` in thaw-hir) resolved a bare identifier's operand
/// type by checking `self.signatures` (the top-level function table)
/// *before* checking whether the name was actually a local variable in
/// `self.scope` -- so inside the dispatcher, `typeof max === "boolean"`
/// resolved `max`'s type from the unrelated top-level `max` function's
/// *signature* instead of the dispatcher's own `boolean | number`
/// parameter, folding the whole comparison to the constant string
/// `"function"` and permanently skipping the boolean branch. Every call
/// then fell through to the number branch, reinterpreting the packed
/// union payload's raw bits for a `boolean` argument as an `f64` --
/// observed as `5e-324` (`Number.MIN_VALUE`, the bit pattern of integer
/// `1`) and a SIGSEGV on repeated calls. Fixed by having the operand
/// type resolution check `self.scope` first, since a local
/// variable/parameter must shadow a same-named top-level function. This
/// only reproduced with a real, large `.d.ts` (lodash's) because that's
/// what supplied the colliding same-named top-level function --
/// isolated `Bool`-discriminator tests without one never hit it.
#[test]
fn bool_discriminated_overload_dispatch_is_not_corrupted_by_a_same_named_top_level_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-bool-overload-name-collision-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("stat-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function random(floating?: boolean): number;\n\
         export declare function random(max: number, floating?: boolean): number;\n\
         export declare function max(a: number, b: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.random = function(a, b) {\n\
             if (typeof a === 'boolean') { return a ? 1 : 0; }\n\
             return a + (b ? 0.5 : 0);\n\
         };\n\
         module.exports.max = function(a, b) { return a > b ? a : b; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { random, max } from "stat-kit";
function main(): void {
    console.log(random(true));
    console.log(random(false));
    console.log(random(5));
    console.log(random(5, true));
    console.log(max(3, 7));
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
        "1\n0\n5\n5.5\n7\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose declared name matches a C standard library
/// math function (`floor`, here -- the real-world trigger: lodash exports
/// `_.floor`/`_.ceil`/`_.round`, and any package declaring one of these
/// pulls in the same shape) used to corrupt *every* `Math.floor`/`Math.
/// ceil`/`Math.round` call made from *any* Fallback JS running in the same
/// program, not just calls to the colliding function itself.
///
/// Root cause: `declare_function` (thaw-llvm) gave every HIR-compiled
/// function -- including this one, generated verbatim from a `.d.ts`
/// declaration by `registry_integration/shims.rs` -- ordinary `external`
/// LLVM linkage, keyed on its bare source name with no mangling
/// (`llvm_symbol_for`). In the final binary that makes it a *global,
/// default-visibility* ELF symbol -- confirmed via `readelf --dyn-syms`
/// showing a `floor` entry in the executable's own dynamic symbol table.
/// Since libm is linked dynamically, Linux's default symbol interposition
/// rule then made *every* reference to `floor` process-wide -- including
/// the one QuickJS-NG's own `Math.floor` builtin makes internally via the
/// PLT -- resolve to *this* native function instead of libm's real
/// `floor(double): double`. Called with a `double` argument (in an XMM
/// register, per the C calling convention) as if it were thaw's own
/// `floor(argsArray: Json): Json` (expecting a pointer in a general-
/// purpose register), it dereferenced garbage and crashed inside
/// `thaw_std::json::ordered_object_fields` -- reproducing as `SIGSEGV` on
/// `Math.floor`/`Math.ceil`/`Math.round` calls anywhere in the program,
/// including ones with nothing to do with the colliding function (this is
/// also the real root cause of the previously-unresolved `_.chunk`
/// SIGSEGV: `_.chunk`'s own implementation calls `Math.floor` internally,
/// and lodash's full `.d.ts` always declares `floor`/`ceil`/`round` too).
///
/// Fixed by giving every HIR-compiled function `internal` LLVM linkage
/// instead: they never need to be called from outside the single LLVM
/// module thaw-llvm compiles the whole program into (intra-module calls
/// resolve directly regardless of linkage), so this closes off the
/// interposition risk entirely without changing how anything actually
/// runs -- confirmed by the existing frame-split IR-text assertions
/// (`define internal ptr @...` instead of `define ptr @...`) and by the
/// full test suite staying green.
#[test]
fn a_fallback_function_named_like_a_libm_function_does_not_corrupt_math_builtins() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-libm-name-collision-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("math-clash-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function floor(n: number): number;\n\
         export declare function useMathFloor(n: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.floor = function(x) { return x - 1000; };\n\
         module.exports.useMathFloor = function(x) { return Math.floor(x); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { floor, useMathFloor } from "math-clash-kit";
function main(): void {
    console.log(floor(5));
    console.log(useMathFloor(4.7));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "-995\n4\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A real `import * as pkg from "pkg"` namespace import, calling a
/// two-parameter (one optional) Fallback function through it
/// (`pkg.major(version, precise?)`), used to be silently hijacked by the
/// unrelated "bare qualifier, no import needed" convenience syntax
/// (`rewrite_qualified_calls`) whenever the chosen namespace alias
/// happened to equal the package's own qualifier -- the natural,
/// idiomatic choice, exactly what real code (`import * as semver from
/// "semver"`) does. That rewrite sends the call to the untyped,
/// single-`argsArray`-parameter Fallback alias instead of the properly
/// typed, arity-dispatching wrapper a plain `import { major }` gets,
/// crashing with "args_json is not a valid JSON array" the moment any
/// argument was passed un-packed. Fixed by having the rewrite skip any
/// qualifier name that's also bound by a real import in the file.
#[test]
fn namespace_import_member_call_is_not_hijacked_by_the_bare_qualifier_rewrite() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-namespace-import-not-hijacked-{}",
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
        r#"import * as verkit from "verkit";
function main(): void {
    console.log(verkit.major("3.2.1"));
    console.log(verkit.major("3.2.1", true));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    // The bare-qualifier rewrite this test guards against only fires for
    // an explicitly `--use`d package (see `qualified_call_rewrites`'s
    // `use_qualifiers` filter in `build_with_link_mode`) -- an
    // auto-resolved-from-the-import package alone doesn't reproduce it.
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3\n3.5\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real zod v4's own `package.d.ts` re-exports its whole API two ways
/// at once: every function flattened to a top-level named export (via
/// `export * from "./v4/classic/external.cjs"`, already resolved by
/// `thaw registry add`'s own install-time flattening into plain
/// `export declare function ...` lines in the same file), *and* the
/// same module star-imported under a name and re-exported as a
/// namespace object (`import * as z from "./v4/classic/external.cjs";
/// export { z, z as default };`) -- so `import { object, string } from
/// "zod"` and `import * as z from "zod"; z.object(...)` are meant to
/// reach the exact same declarations. This does *not* exercise a
/// `declare namespace` block (a genuinely different, still-unsupported
/// shape -- a namespace whose members are declared *inside* it, not a
/// star-import of an already-flattened sibling module) -- confirmed
/// via a synthetic reproduction of zod's literal shape that this one
/// already works end-to-end: thaw-bridge's parser simply has no
/// declaration named `z` to classify at all (it's only ever bound by
/// an `import *`, never a real top-level `function`/`class`/
/// `interface`), so it's silently skipped, and the *user's own*
/// `import * as z from "case-kit3"` is an entirely ordinary namespace
/// import of the package's already-flattened export table -- unrelated
/// machinery already handles it (see `namespace_import_member_call_is_
/// not_hijacked_by_the_bare_qualifier_rewrite` above).
#[test]
fn a_namespace_reexported_from_the_packages_own_flattened_module_works_like_a_plain_namespace_import()
 {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-self-reexported-namespace-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit3");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        // Mirrors real zod's exact `index.d.ts` shape post-flattening:
        // an unresolved self-referencing `import *`/`export *`/
        // `export { z, z as default }` trio (the relative path is
        // never actually followed -- there is no such file here --
        // exactly like thaw-bridge's single-file parser never follows
        // it for real zod either), followed by the already-flattened
        // top-level declarations that path's `export *` used to stand
        // for.
        "import * as z from \"./lib\";\n\
         export * from \"./lib\";\n\
         export { z, z as default };\n\
         \n\
         export declare function double(value: number): number;\n\
         export declare function greet(name: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.double = function(value) { return value * 2; };\n\
         module.exports.greet = function(name) { return 'hi ' + name; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as z from "case-kit3";
function main(): void {
    console.log(z.double(21));
    console.log(z.greet("Alice"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\nhi Alice\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The same self-referential namespace shape as the test above
/// (`import * as z`), but imported by *name* instead --
/// `import { z } from "case-kit4";`, real-world shape: zod's own docs
/// show both `import * as z from "zod"` and `import { z } from "zod"`
/// as equivalent, since both reach the identical namespace object. Used
/// to fail outright at the module-graph level (`` `zod` has no export
/// named `z` ``): a named import resolves a specific key from the
/// package's flat export table, and `z` was never in it at all --
/// there's no function/class/interface actually named `z` anywhere in
/// the flattened `.d.ts` (it's only ever bound by the package's own
/// internal `import * as z`), unlike a namespace import, which just
/// hands over the *whole* export table under whatever local name the
/// user chose, with no per-name lookup needed at all.
///
/// Fixed with `thaw_bridge::self_referential_namespace_aliases`
/// (recognizing `import * as z from "./local"; export { z, z as
/// default };` in a package's own `.d.ts`) threaded through as a new
/// `ExternalNamespaceAliases` map (thaw-cli's `shims.rs` / `build.rs` /
/// `module_graph.rs`): a named import (or a same-file `export { z }
/// from "pkg";` re-export) of exactly one of these recognized names
/// now resolves the same way a namespace import already did, instead
/// of failing the ordinary single-symbol lookup.
#[test]
fn a_named_import_of_a_self_referential_namespace_alias_works_like_a_namespace_import() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-named-self-reexported-namespace-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit4");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "import * as z from \"./lib\";\n\
         export * from \"./lib\";\n\
         export { z, z as default };\n\
         \n\
         export declare function double(value: number): number;\n\
         export declare function greet(name: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { double: function(value) { return value * 2; }, greet: function(name) { return 'hi ' + name; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { z } from "case-kit4";
function main(): void {
    console.log(z.double(21));
    console.log(z.greet("Alice"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\nhi Alice\n");
    let _ = std::fs::remove_dir_all(dir);
}
