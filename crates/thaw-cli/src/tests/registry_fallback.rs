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
        "export declare function clock(seed?: unknown): Promise<unknown>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.clock = function(seed) {\n\
             return { toString: function() { return 'tick'; } };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { clock } from "clock-kit";
function main(): void {
    console.log(clock());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"H:3\"\n");
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
        "\"no-args-ok\"\n"
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
        "\"wrapped(thing:gadget)\"\n{}\n"
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
        "\"optional-str\"\n"
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"str,num\"\n");
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
        "\"v\"\n\"h\"\n\"v\"\n42\n"
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
        "\"thing:root.a.b\"\n"
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

/// `coerce_to_declared`'s missing symmetric case to its own `Json`
/// branch just above it: a dynamic method call with no `JsValue` hint
/// (`lower_dynamic_value_method_call`'s own default) always comes back
/// `Json`-typed, even when the caller's own declared slot is a concrete
/// scalar -- real trigger found while finishing the hono `app.get` arc:
/// `const body: string = await res.text()` used to fail outright
/// ("value has type Json, expected Str"), with no way to consume the
/// result except keeping it `JsValue`-typed and decoding it manually
/// later. Fixed by reusing the exact `JsonAsNumber`/`JsonAsString`/
/// `JsonAsBool` nodes `dictionary_value_from_json` (objects.rs) already
/// builds for the identical "decode a Json value into its declared
/// scalar type" problem elsewhere -- already fully supported by type
/// inference and both codegen backends, so no new HIR node or codegen
/// path was needed, just a new branch in `coerce_to_declared` itself.
///
/// Exercises all three scalar targets (`number`, `boolean`, `string`)
/// against one holder object whose methods all return plain data with
/// no annotation anywhere on the call site itself.
#[test]
fn a_dynamic_method_calls_json_result_decodes_into_a_declared_scalar_type() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-json-to-scalar-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("scalar-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         num: function() { return 42; }, \
         flag: function() { return true; }, \
         text: function() { return 'hi'; } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "scalar-kit";
function main(): void {
    const h: JsValue = makeHolder();
    const n: number = h.num();
    const b: boolean = h.flag();
    const s: string = h.text();
    console.log(n);
    console.log(b);
    console.log(s);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\ntrue\nhi\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `JsValue`-returning Fallback function's result passed straight into
/// *another* Fallback function's `Json`-declared parameter, both
/// unannotated at the call site (no intermediate `: JsValue`/`: Json`
/// binding) -- real-world example: real uuid's `stringify(parse(id))`,
/// where `parse`'s return type (`NonSharedArrayBuffer`) and `stringify`'s
/// parameter type (`Uint8Array`) are both unresolved reference types
/// thaw-bridge classifies the same "unclassified npm type" way `parse`'s
/// gets treated as `JsValue` (a return value) while `stringify`'s gets
/// treated as `Json` (a parameter) -- and `stringify` also has a second,
/// omittable optional parameter, so calling it with just one argument
/// (`stringify(bytes)`) routes through thaw-hir's omitted-parameter-mask
/// wrapper mechanism, an *ordinary* compiled function call like any
/// other, not a manually-built dynamic-call intrinsic.
///
/// Used to crash LLVM's own module verifier at build time ("Call
/// parameter type does not match function signature!") -- `coerce_to_
/// declared` (thaw-hir) already passes a `JsValue` through unchanged
/// into a `Json`-declared slot (relying on downstream codegen to
/// recognize the mismatch and thread the real handle through instead of
/// a raw, mistyped integer), but `build_call_with` (the codegen path an
/// *ordinary* function call like this one takes) never did any such
/// recognition at all -- only the JSON-args-array-construction call
/// sites did (guarded by `compiling_quickjs_dynamic_arguments`, which
/// this path never set). Fixed by having `build_call_with` itself detect
/// the same "expected a pointer (Json/Dictionary's own representation),
/// got a raw int (actually a `JsValue` handle)" mismatch per argument,
/// using `compile_dynamic_value_placeholder`'s own encoding via a new
/// gate-free variant (safe here since an N-API/native-addon target never
/// reaches this call path at all, unlike the gated call sites).
#[test]
fn a_jsvalue_returning_functions_result_passed_into_another_functions_json_parameter_works() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-into-json-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("buffer-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function parse(id: string): NonSharedArrayBuffer;\n\
         export declare function stringify(arr: Uint8Array, offset?: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         parse: function(id) { return { tag: id, length: 16 }; }, \
         stringify: function(arr, offset) { return 'stringified:' + arr.tag; } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { parse, stringify } from "buffer-kit";
function main(): void {
    const bytes = parse("hello");
    console.log(bytes.length);
    const s: string = stringify(bytes);
    console.log(s);
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
        "16\nstringified:hello\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A factory value bound directly to a name (`export declare const NAME:
/// SomeCallableInterface;`) instead of declared `function` -- the shape
/// drizzle-orm's `sqliteTable`/`pgTable` use (`SQLiteTableFn`/`PgTableFn`,
/// an interface with one call signature per overload). Confirms it's
/// reachable and callable end to end: `thaw_bridge::parse_dts` synthesizes
/// a `DtsFunction` from the interface's call signature(s), which flows
/// through the same `Classification::Fallback` shim-generation path as any
/// other npm function.
#[test]
fn a_declare_const_bound_to_a_callable_interface_is_reachable_and_callable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callable-const-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("table-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Factory {\n\
         \x20\x20\x20\x20(name: string, count: number): string;\n\
         }\n\
         export declare const factory: Factory;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         factory: function(name, count) { return name + ':' + count; } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { factory } from "table-kit";
function main(): void {
    const result: string = factory("users", 3);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "users:3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn callable_object_properties_chain_through_live_javascript_values() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callable-object-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("style-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Style { (...text: unknown[]): string; readonly upper: Style; }\n\
         declare const style: Style;\nexport default style;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function style(value) { return String(value).toUpperCase(); }\n\
         style.upper = style;\nmodule.exports = { default: style };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import style from "style-kit";
function main(): void { console.log(style.upper("hello")); }
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"HELLO\"\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn non_callable_named_and_singleton_exports_are_reachable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-value-exports-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let constants = registry.join("constant-kit");
    let singleton = registry.join("mime-kit");
    std::fs::create_dir_all(&constants).unwrap();
    std::fs::create_dir_all(&singleton).unwrap();
    std::fs::write(constants.join("package.d.ts"), "export declare const NIL: string;\nexport declare const MAX: string;\n").unwrap();
    std::fs::write(constants.join("bundle.js"), "module.exports = { NIL: 'zero-id', MAX: 'max-id' };\n").unwrap();
    std::fs::write(singleton.join("package.d.ts"), "export declare class Mime { getType(path: string): string | null; }\ndeclare const mime: Mime;\nexport = mime;\n").unwrap();
    std::fs::write(singleton.join("bundle.js"), "module.exports = { getType: function(path) { return path.endsWith('.txt') ? 'text/plain' : null; } };\n").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, r#"import { NIL, MAX } from "constant-kit";
import mime from "mime-kit";
function main(): void {
    console.log(NIL + ":" + MAX);
    console.log(JSON.stringify(mime.getType("note.txt")));
}"#).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &["constant-kit".into(), "mime-kit".into()]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "zero-id:max-id\n\"text/plain\"\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn generic_fallback_callback_alias_is_contextually_typed() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-callback-alias-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "type Iterator<T, R> = (value: T, index: number, values: T[]) => R;\nexport declare function map<T, R>(values: T[], iterator: Iterator<T, R>): R[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { map: function(values, iterator) { return values.map(iterator); } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { map } from 'callback-kit'; function main(): void { map([1, 2, 3], value => value * 2); console.log('ok'); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &["callback-kit".into()]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn discarded_dynamic_method_results_do_not_require_json_serialization() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-discarded-dynamic-result-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("listener-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function make(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "class Emitter { constructor() { this.events = {}; } on(name, callback) { (this.events[name] || (this.events[name] = [])).push(callback); return this; } emit(name, value) { (this.events[name] || []).forEach(function(callback) { callback(value); }); } } class Listener extends Emitter { constructor() { super(); this.self = this; } fire() { this.emit('value', this); return this; } } class Outer { constructor() { this.target = new Listener(); } fire() { this.target.fire(); return this; } } ['on'].forEach(function(name) { Outer.prototype[name] = function() { return this.target[name].apply(this.target, arguments); }; }); module.exports.make = function() { return new Outer(); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { make } from 'listener-kit'; function main(): void { const listener: JsValue = make(); listener.on('value', (value: JsValue): void => { console.log('called'); }); listener.fire(); }\n",
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
        &["listener-kit".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "called\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn synchronous_void_native_callback_returns_undefined_to_javascript() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-sync-void-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function run(callback: () => void): boolean;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { run: function(callback) { return callback() === undefined; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { run } from 'callback-kit'; function main(): void { console.log(run((): void => {})); }\n",
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
        &["callback-kit".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fallback_method_signature_contextually_types_callbacks_and_type_only_imports() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-contextual-method-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("web-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export type { Context } from './context';\n\
         export declare class Hono {\n\
         \x20 constructor();\n\
         \x20 get(path: string, handler: (context: JsValue) => Json): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Hono() {} Hono.prototype.get = function(path, handler) { return handler({ text: function(value) { return value; } }); }; module.exports = { Hono: Hono };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Hono, Context } from "web-kit";
function main(): void {
    const app = new Hono();
    app.get("/", (c) => { const value: string = c.text("Hello Thaw"); console.log(value); return JSON.parse("{}"); });
    app.get("/typed", (c: Context) => { const value: string = c.text("Typed"); console.log(value); return JSON.parse("{}"); });
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &["web-kit".into()]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "Hello Thaw\nTyped\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fallback_object_callback_is_contextually_typed_and_runs_as_native_code() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-contextual-object-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("server-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export type Handler<T = unknown> = (request: T, toolkit: T, error?: T) => T;\n\
         export interface Route<T = unknown> { method: string; path: string; handler?: Handler<T> | object | undefined; }\n\
         export declare class Server { route<T = unknown>(route: Route<T> | Route<T>[]): void; }\n\
         export declare function server(): Server;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Server() {} Server.prototype.route = function(route) { console.log(route.handler({ path: route.path }, { response: function(value) { return value; } })); }; function server() { return new Server(); } module.exports = { Server: Server, server: server };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as Kit from "server-kit";
function main(): void {
    const server = Kit.server();
    server.route({ method: "GET", path: "/jit", handler: (request, toolkit) => toolkit.response(request.path) });
}"#,
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
        &["server-kit".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "/jit\n");
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn imports_node_builtin_object_values() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-node-object-values-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { Buffer } from "node:buffer";
           function main(): void { console.log(Number(Buffer.byteLength("thaw"))); }"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "4\n");
    let _ = std::fs::remove_dir_all(dir);
}
