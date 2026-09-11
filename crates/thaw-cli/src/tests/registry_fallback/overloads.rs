/// A Fallback function (forced by an unclassifiable `unknown`-typed
/// parameter, real-world example: uuid's `v4`/`validate`) with several
/// *trailing optional* parameters. Regression coverage for two bugs found
/// together while getting uuid itself to build: `typed_dynamic_declaration`
/// used to give up on a typed wrapper entirely the moment any one parameter
/// was unclassifiable, and even once that was fixed, its generated wrapper
/// only guarded the *last* optional slot with `!== undefined`, so the
/// type-narrowing pass left every earlier optional parameter still
/// `Optional(...)` where the underlying call needed it unwrapped.
#[test]
fn fallback_function_with_multiple_optional_unknown_params_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-multi-optional-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("id-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function make(prefix?: unknown, suffix?: unknown, length?: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.make = function(prefix, suffix, length) {\n\
             var p = prefix === undefined ? '' : String(prefix);\n\
             var s = suffix === undefined ? '' : String(suffix);\n\
             var n = length === undefined ? 0 : length;\n\
             return p + 'id' + s + ':' + n;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { make } from "id-kit";
function main(): void {
    console.log(make());
    console.log(make("a"));
    console.log(make("a", "b"));
    console.log(make("a", "b", 5));
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
        "id:0\naid:0\naidb:0\naidb:5\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Two `.d.ts` overloads for the same Fallback name (real-world example:
/// `ms`'s `(value: number, options?)` / `(value: string)`) can each
/// independently produce a valid typed wrapper. Regression coverage for a
/// bug introduced alongside the fix above: broadening which parameter types
/// count as classifiable meant a second, narrower overload could newly
/// succeed too and silently clobber the first (and better -- it covers both
/// arities) overload's registration, since call-site rewriting just kept
/// whichever overload was processed last.
#[test]
fn first_matching_overload_wins_for_a_fallback_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-order-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("format-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function format(value: number, upper?: boolean): string;\n\
         export declare function format(value: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.format = function(value, upper) {\n\
             if (typeof value === 'string') { return '[' + value + ']'; }\n\
             var text = String(value) + 'px';\n\
             return upper ? text.toUpperCase() : text;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { format } from "format-kit";
function main(): void {
    console.log(format(12));
    console.log(format(12, true));
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
    // If the second (narrower, single required-param) overload had won
    // instead, `format(12, true)` -- 2 arguments -- would be a compile
    // error, since that overload's own wrapper only ever accepts 1.
    assert_eq!(String::from_utf8_lossy(&result.stdout), "12px\n12PX\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose overloads are discriminated by *arity
/// range*, not `typeof` -- real-world example: uuid's `v4(options?):
/// string` (0-1 args) alongside its generic buffer-output
/// `v4<TBuf extends Uint8Array = Uint8Array>(options, buf, offset?):
/// TBuf` (2-3 args), previously unreachable no matter what a real call
/// passed, since "first successful overload wins" only ever exposed the
/// first. Exercises the full pipeline end to end (`generate_registry_
/// shims`'s new per-overload declarations plus `class_methods.rs`'s new
/// bare-call rewrite), not just the rewrite function in isolation --
/// this is the test that would have caught the original bug.
#[test]
fn fallback_function_overload_is_picked_by_call_site_arity() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-arity-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("id-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeId(seed?: number): string;\n\
         export declare function makeId(seed: number, salt: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeId = function(seed, salt) {\n\
             if (salt !== undefined) { return seed * 1000 + salt; }\n\
             return 'id-' + (seed === undefined ? 0 : seed);\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeId } from "id-kit";
function main(): void {
    console.log(makeId());
    console.log(makeId(5));
    console.log(makeId(5, 7));
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
    // If the 2-argument call had stayed pinned to the first (0-1 arg)
    // overload's declaration instead, this would be a compile error
    // (too many arguments), not a wrong runtime value.
    assert_eq!(String::from_utf8_lossy(&result.stdout), "id-0\nid-5\n5007\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose `.d.ts` signature ends in a rest parameter
/// (real-world example: `clsx(...inputs: ClassValue[]): string`).
/// `typed_dynamic_declaration` used to ignore `rest_param` entirely and
/// declare a fixed 0-argument extern signature, so any real (non-empty)
/// call became an arity-mismatch compile error. Regression coverage for
/// `typed_dynamic_rest_declaration`, which instead declares one extern
/// signature per call-site argument count actually observed in the user's
/// own source and dispatches on the wrapper's own `...rest` array length.
#[test]
fn fallback_function_with_a_rest_parameter_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-rest-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("join-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function joinValues(...inputs: unknown[]): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.joinValues = function() {\n\
             var parts = [];\n\
             for (var i = 0; i < arguments.length; i++) {\n\
                 if (arguments[i]) { parts.push(String(arguments[i])); }\n\
             }\n\
             return parts.join(' ');\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { joinValues } from "join-kit";
function main(): void {
    console.log(joinValues("a", "b"));
    console.log(joinValues("a", false, "b", null, "c"));
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
        "a b\na b c\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn generic_rest_callbacks_are_inferred_and_passed_without_array_marshalling() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-rest-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("order-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function orderBy<T>(values: T[], ...iteratees: Array<(value: T) => number>): T[];\n\
         export declare function orderBy<T extends object>(values: T, ...iteratees: Array<(value: T[keyof T]) => number>): Array<T[keyof T]>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orderBy = function(values) { \
         var iteratees = Array.prototype.slice.call(arguments, 1); \
         var result = Array.isArray(values) ? values : Object.values(values); \
         iteratees.forEach(function(iteratee) { result.forEach(iteratee); }); \
         return result; \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { orderBy } from "order-kit";
function main(): void {
    let total: number = 0;
    const result = orderBy([3, 1, 2], value => { total = total + value; return value; });
    console.log(result[0]);
    console.log(result.join(','));
    const objectResult = orderBy({ a: 4, b: 2 }, value => { total = total + value; return value; });
    console.log(objectResult[0]);
    console.log(objectResult.join(','));
    console.log(total);
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
        "3\n3,1,2\n4\n4,2\n12\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function (any single param/return referencing its
/// own unconstrained type parameter classifies Fallback, since that's an
/// unresolved reference as far as `classify` is concerned) declared as
/// returning `void`. The generic branch of `typed_dynamic_declaration`
/// used to render `return {base_symbol}__arity_N(...);` unconditionally,
/// but thaw-hir rejects `return <a void call>;` for a `void`-declared
/// function (only a bare `return;`), so any real call was a compile
/// error. Regression coverage for splitting the call and the return into
/// separate statements when the declared return is `void`.
#[test]
fn generic_fallback_function_with_void_return_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-generic-void-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("seen-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function markSeen<T>(value: T): void;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "var seen = [];\nmodule.exports.markSeen = function(value) { seen.push(value); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { markSeen } from "seen-kit";
function main(): void {
    markSeen("a");
    markSeen(1);
    console.log("done");
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "done\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function with a parameter type this classifier
/// can't resolve at all (a reference to a type alias never declared
/// anywhere in the file, e.g. because it lives in some other file a
/// simpler package never pulled in -- real-world example: lodash's
/// `uniq<T>(array: List<T> | null | undefined): T[]`, `List` being one
/// of lodash's own internal aliases). The generic branch used to reject
/// the whole declaration outright whenever any parameter didn't already
/// render as one of a few known-safe forms, falling all the way back to
/// the bare `(argsArray: Json): Json` passthrough shim -- silently wrong
/// for an ordinary single-argument call like `firstOf(someArray)`, whose
/// real argument would be passed as the whole *args array* instead.
/// Regression coverage for substituting `Json` for just that one
/// parameter instead of giving up on the whole function.
///
/// `firstOf`'s return is bare `T`, preserved by the same generic branch
/// as a real type parameter (see
/// `generic_fallback_function_with_return_only_type_parameter_infers_it_from_a_let_annotation`),
/// so this specific call needs a `let`/`const` annotation to give
/// thaw-hir something to infer `T` from -- an unannotated, directly
/// nested call has no argument that mentions `T` either, once `items`
/// itself was substituted to `Json`.
#[test]
fn generic_fallback_function_with_unresolvable_param_type_passes_it_through_as_json() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-generic-json-passthrough-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("first-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function firstOf<T>(items: List<T> | null): T;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.firstOf = function(items) { return items && items.length ? items[0] : null; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { firstOf } from "first-kit";
function main(): void {
    const items: Json = JSON.parse("[10, 20, 30]");
    const first: number = firstOf(items);
    console.log(first);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "10\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose type parameter appears *only* in
/// the return position -- real-world example: nanoid's own `nanoid<Type
/// extends string>(size?: number): Type`. thaw-hir has no argument to
/// infer `Type` from at all here, but can still infer it from a
/// `let`/`const` declaration's own type annotation as a fallback (see
/// `infer_generic_type_tuple`/`lower_expr_with_expected_type` in
/// thaw-hir), so the generic branch keeps this one type parameter
/// declared (unlike a type parameter with no remaining occurrence
/// anywhere, dropped as dead syntax) and renders the real return type
/// instead of the usual hardcoded `JsValue`.
#[test]
fn generic_fallback_function_with_return_only_type_parameter_infers_it_from_a_let_annotation() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-return-only-generic-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("id-gen-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeId<Type extends string>(size?: number): Type;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeId = function(size) {\n\
             var n = size === undefined ? 3 : size;\n\
             var out = '';\n\
             for (var i = 0; i < n; i++) { out += 'x'; }\n\
             return out;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeId } from "id-gen-kit";
function main(): void {
    const id: string = makeId();
    console.log(id.length);
    const short: string = makeId(5);
    console.log(short.length);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3\n5\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose *return* type can't be classified at all --
/// real-world example: mime's `getType(path: string): string | null`
/// alongside a hypothetical sibling overload whose return type this
/// crude a classifier can't yet describe, modeled here with a `Promise`
/// return (still explicitly out of scope -- see `classify_ts_type`'s
/// `TsTypeRef` doc comment). `typed_dynamic_declaration` used to give up
/// on the whole declaration the moment the return type alone didn't
/// classify (even though every parameter already passes through as
/// `Json` for exactly this reason), falling all the way back to the
/// bare untyped `(argsArray: Json): Json` passthrough shim -- silently
/// wrong for any call that isn't already packing its own arguments into
/// one array itself, the same failure mode as an unclassifiable
/// parameter. Regression coverage for treating an unclassifiable return
/// the same way a callback-typed return already was: a `JsValue`
/// handle, usable even without dedicated support for whatever real
/// shape it holds.
///
/// (Not namespace-qualified, unlike this test's original form: a
/// namespace-qualified reference to an interface/type-alias, even an
/// empty one, now resolves -- see `extract_interface_decls`/
/// `extract_type_alias_decls` and `classify_ts_type`'s own doc comment
/// on `TsQualifiedName` -- so it no longer demonstrates "unresolvable"
/// at all.)
#[test]
fn fallback_function_with_an_unresolvable_return_type_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-unresolvable-return-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clock-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function clock(seed?: unknown): unknown;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.clock = function(seed) { return 'tick'; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { clock } from "clock-kit";
function main(): void {
    console.log(String(clock()));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "tick\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `export default function name(...): T;` alongside a second, separate
/// named export in the same `.d.ts` -- real-world example: leven's own
/// `export default function leven(...): number;` plus its named
/// `export function closestMatch(...): string | undefined;`.
/// `commonjs_export_name` used to recognize only CommonJS's `export = x;`,
/// so a package's "default" binding fell back to working only when the
/// package had *exactly one* function total -- true for most
/// single-purpose packages, but not one exporting more than one function
/// this way, where `import leven from "leven"` had nothing telling it
/// which of the two `leven` itself actually names.
#[test]
fn esm_default_export_alongside_a_second_named_export_resolves_the_default_import() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-esm-default-plus-named-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("distance-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export default function distance(a: string, b: string): number;\n\
         export function closestMatch(target: string, candidates: string[]): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.distance = function(a, b) { return a === b ? 0 : 1; };\n\
         module.exports.closestMatch = function(target, candidates) { return candidates[0]; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import distance from "distance-kit";
function main(): void {
    console.log(distance("cat", "cat"));
    console.log(distance("cat", "cow"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n1\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose parameter (or an object argument's own
/// field) is a union type -- real-world example: camelcase's own
/// `camelCase(input: string | readonly string[], options?: {
/// pascalCase?: boolean, ... }): string`. Marshaling a dynamic call's
/// argument to JSON had no case for `HirType::Union` at either the
/// top-level-argument or the nested-object-field layer, so passing a
/// plain string (matching the union's *first* member, but still a
/// union statically) failed outright rather than only breaking for
/// values that actually needed the second member.
#[test]
fn fallback_function_with_a_union_typed_argument_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-union-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        // `separator` is itself a union field *inside* the options
        // object, exercising the object-field JSON-marshaling path
        // separately from `input`'s own top-level union.
        "export interface CaseOptions { separator?: string | boolean; }\n\
         export declare function toCase(input: string | readonly string[], options?: CaseOptions): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.toCase = function(input, options) {\n\
             var s = Array.isArray(input) ? input.join('-') : input;\n\
             var sep = options && options.separator;\n\
             return typeof sep === 'string' ? s.split('-').join(sep) : s;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { toCase } from "case-kit";
function main(): void {
    console.log(toCase("foo"));
    console.log(toCase(["a", "b"], { separator: "_" }));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert!(manifest["quickjs_reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| {
            reason["kind"] == "registry-fallback"
                && reason["package"] == "case-kit"
                && reason["function"] == "toCase"
                && reason["line"] == 3
        }));
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "foo\na_b\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose parameter/return type is `Date`, generic
/// over it (`<DateType extends Date>`) -- real-world example: date-fns's
/// `format<DateType extends Date>(date: DateType | number | string,
/// formatStr: string): string` and `addDays<DateType extends Date>(date:
/// ..., amount: number): DateType`. Exercises the whole round trip: a
/// Thaw-native `Date` (an object with a `timestamp` field) argument must
/// arrive on the QuickJS-NG side as a real `instanceof Date` (this fake
/// package's `format`/`addDays` both throw/misbehave otherwise), and a
/// real `Date` returned from QuickJS-NG must come back as that same
/// native shape, usable as a plain `Date` argument to a later call. Only
/// UTC-based formatting is asserted (never a local-time token like `HH`)
/// so this doesn't depend on the host's time zone.
#[test]
fn fallback_function_with_a_generic_date_argument_and_return_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-date-generic-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("temporal-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function format<DateType extends Date>(date: DateType | number | string, formatStr: string): string;\n\
         export declare function addDays<DateType extends Date>(date: DateType | number | string, amount: number): DateType;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function pad(n) { return n < 10 ? '0' + n : '' + n; }\n\
         module.exports.format = function(date, formatStr) {\n\
             if (!(date instanceof Date)) throw new Error('Invalid time value');\n\
             return date.getUTCFullYear() + '-' + pad(date.getUTCMonth() + 1) + '-' + pad(date.getUTCDate());\n\
         };\n\
         module.exports.addDays = function(date, amount) {\n\
             if (!(date instanceof Date)) throw new Error('Invalid time value');\n\
             return new Date(date.getTime() + amount * 86400000);\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { format, addDays } from "temporal-kit";
function main(): void {
    const d = new Date(2024, 0, 15);
    console.log(format(d, "yyyy-MM-dd"));
    const later: Date = addDays(d, 10);
    console.log(format(later, "yyyy-MM-dd"));
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
        "2024-01-15\n2024-01-25\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Two `.d.ts` overloads spanning *different, overlapping* arity ranges
/// (real-world example: jsonwebtoken's own `verify(token, secretOrKey,
/// options?)` -- arity `(2, 3)` -- alongside its own `verify(token,
/// secretOrKey, options?, callback?)` -- arity `(2, 4)`), where a 2-argument
/// call's arguments can't be scored against either overload at all (one
/// argument's type isn't tracked, matching the extremely common `const x =
/// someDynamicCall(); f(x, ...)` shape; the other is declared `Json`, which
/// always scores 0 regardless of the actual argument). Regression coverage
/// for a bug where the "are these tied candidates interchangeable" tie-break
/// compared each candidate's *entire* declared parameter vector -- including
/// trailing parameters neither candidate was actually given -- so two
/// candidates whose *provided* parameter slots agree exactly still failed
/// the check purely because their *total* arity differs, leaving `selected
/// = None` and falling through to the first-declared overload's own arity
/// requirement (here, a 3-argument-minimum overload that must be listed
/// first for `.d.ts` overload resolution's own "first overload wins"
/// default to pick it) instead of either 2-argument-accepting one.
#[test]
fn overlapping_arity_ranges_tie_break_only_compares_provided_argument_slots() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overlapping-arity-tie-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("verify-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function verifyLike(token: string, secret: Json, options: Json): string;\n\
         export declare function verifyLike(token: string, secret: Json, options?: Json): string;\n\
         export declare function verifyLike(token: string, secret: Json, options?: Json, callback?: Json): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.verifyLike = function(token, secret) {\n\
             return token + ':' + secret;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { verifyLike } from "verify-kit";
function main(): void {
    const raw: Json = JSON.parse('{"token":"abc"}');
    const token = raw.token;
    console.log(verifyLike(token, "shh"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "abc:shh\n");
    let _ = std::fs::remove_dir_all(dir);
}
