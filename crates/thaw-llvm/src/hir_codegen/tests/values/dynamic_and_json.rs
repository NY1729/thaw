#[test]
fn compiles_native_null_literals() {
    let source = r#"
        function getNull(): null {
            console.log("get null");
            return null;
        }
        function main(): void {
            const value: null = null;
            console.log(value);
            console.log(typeof value);
            console.log(null === null);
            console.log(null === undefined);
            console.log(null !== undefined);
            console.log(getNull() == undefined);
            console.log(getNull() != undefined);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_null"),
        "null\nobject\ntrue\nfalse\ntrue\nget null\ntrue\nget null\nfalse\n"
    );
}

/// `String`/`Number`/`Boolean` referenced *bare* (not called) used to
/// fail to compile ("unknown variable `String`") -- only their
/// call-position coercion form (`String(x)`) was recognized at all.
/// Real JS treats them as ordinary first-class function values
/// (`typeof String === 'function'`, `schema.name === String`), the
/// exact idiom real `mongoose` schemas use (`{ name: String, age:
/// Number }`). Fixed by bridging a bare reference to the real native
/// `String`/`Number`/`Boolean` global QuickJS already provides, the
/// same "dynamic global" mechanism `process`/`crypto`/`Atomics`/
/// `AbortSignal` already use -- doesn't interfere with the existing
/// call-position intrinsic, which is intercepted earlier by
/// `lower_call` reading the callee name directly and never reaches
/// this bare-identifier path.
#[test]
fn bare_string_number_boolean_are_real_first_class_function_values() {
    let source = r#"
        function main(): void {
            console.log(typeof String);
            console.log(typeof Number);
            console.log(typeof Boolean);
            const captured = String;
            console.log(captured === String);
            console.log(String("still works"));
            console.log(Number("42") + 1);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "bare_string_number_boolean_values"),
        "function\nfunction\nfunction\ntrue\nstill works\n43\n"
    );
}

#[test]
fn compiles_native_nullable_values() {
    let source = r#"
        interface Box { value: number | null; label: string; }
        interface Item {
            value: number;
            maybe: number | null;
            run: (value: number) => number;
            runMaybe: (present: boolean) => number | null;
        }
        function maybe(present: boolean): number | null {
            if (present) return 4;
            return null;
        }
        function fallback(): number {
            console.log("fallback");
            return 9;
        }
        function maybeItem(present: boolean): Item | null {
            if (present) return {
                value: 5,
                maybe: null,
                run: (value: number) => value * 2,
                runMaybe: (present: boolean) => maybe(present)
            };
            return null;
        }
        function maybeText(present: boolean): string | null {
            if (present) return "thaw";
            return null;
        }
        function maybeCallback(present: boolean): ((value: number) => number) | null {
            if (present) return (value: number) => value + 1;
            return null;
        }
        function maybeNullableCallback(present: boolean): (() => number | null) | null {
            if (present) return () => maybe(false);
            return null;
        }
        function narrowed(value: number | null): number {
            if (value !== null) return value + 2;
            return 0;
        }
        function guarded(value: number | null): number {
            if (value === null) return 1;
            return value * 2;
        }
        async function delayed(present: boolean): Promise<string | null> {
            await sleep(1);
            if (present) return "ok";
            return null;
        }
        async function main(): Promise<void> {
            console.log(maybe(true));
            console.log(maybe(false));
            console.log(maybe(true) ?? fallback());
            console.log(maybe(false) ?? fallback());
            console.log(maybe(false) === null);
            console.log(maybe(true) === null);
            console.log(maybe(false) == undefined);
            console.log(maybe(true) == undefined);
            console.log(typeof maybe(true));
            console.log(typeof maybe(false));
            let value: number | null = null;
            console.log(value);
            value = 6;
            console.log(value);
            value = null;
            console.log(value ??= fallback());
            console.log(value ??= fallback());
            const box: Box = { value: null, label: "box" };
            console.log(box.value);
            console.log(box.label);
            console.log(maybeItem(true)?.value);
            console.log(maybeItem(false)?.value);
            console.log(maybeItem(true)?.maybe);
            console.log(maybeItem(false)?.maybe);
            console.log(maybeItem(true)?.run(3));
            console.log(maybeItem(false)?.run(fallback()));
            console.log(maybeItem(true)?.["runMaybe"](false));
            console.log(maybeItem(false)?.["runMaybe"](true));
            console.log(maybeText(true)?.length);
            console.log(maybeText(false)?.toUpperCase());
            console.log(maybeCallback(true)?.(7));
            console.log(maybeCallback(false)?.(fallback()));
            console.log(maybeNullableCallback(true)?.());
            console.log(maybeNullableCallback(false)?.());
            console.log(narrowed(3));
            console.log(narrowed(null));
            console.log(guarded(4));
            console.log(guarded(null));
            console.log(await delayed(true));
            console.log(await delayed(false));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_nullable"),
        "4\nnull\n4\nfallback\n9\ntrue\nfalse\ntrue\nfalse\nnumber\nobject\nnull\n6\nfallback\n9\n9\nnull\nbox\n5\nundefined\nnull\nundefined\n6\nundefined\nnull\nundefined\n4\nundefined\n8\nundefined\nnull\nundefined\n5\n0\n8\n1\nok\nnull\n"
    );
}

#[test]
fn compiles_native_three_way_nullish_values() {
    let source = r#"
        interface Box { value: number | null | undefined; label: string; }
        interface Item { value: number; }
        function maybe(kind: number): number | null | undefined {
            if (kind === 1) return 4;
            if (kind === 2) return null;
            return undefined;
        }
        function show(value: number | null | undefined): void {
            console.log(value);
        }
        function maybeItem(kind: number): Item | null | undefined {
            if (kind === 1) return { value: 6 };
            if (kind === 2) return null;
            return undefined;
        }
        function narrowed(value: number | null | undefined): number {
            if (value != null) return value + 1;
            return 0;
        }
        function guarded(value: number | null | undefined): number {
            if (value == null) return 0;
            return value * 2;
        }
        async function delayed(kind: number): Promise<number | null | undefined> {
            await sleep(1);
            return maybe(kind);
        }
        async function main(): Promise<void> {
            show(maybe(1));
            show(maybe(2));
            show(maybe(3));
            console.log(maybe(1) ?? 9);
            console.log(maybe(2) ?? 9);
            console.log(maybe(3) ?? 9);
            console.log(maybe(2) === null);
            console.log(maybe(3) === null);
            console.log(maybe(3) === undefined);
            console.log(maybe(2) === undefined);
            console.log(maybe(2) == undefined);
            console.log(maybe(3) == null);
            console.log(typeof maybe(1));
            console.log(typeof maybe(2));
            console.log(typeof maybe(3));
            console.log(maybeItem(1)?.value);
            console.log(maybeItem(2)?.value);
            console.log(maybeItem(3)?.value);
            let value: number | null | undefined = null;
            console.log(value);
            value = 8;
            console.log(value);
            value = undefined;
            console.log(value);
            console.log(value ??= 11);
            console.log(value ??= 12);
            console.log(value + 1);
            console.log(narrowed(maybe(1)));
            console.log(narrowed(maybe(2)));
            console.log(narrowed(maybe(3)));
            console.log(guarded(maybe(1)));
            console.log(guarded(maybe(2)));
            console.log(guarded(maybe(3)));
            const box: Box = { value: null, label: "box" };
            console.log(box.value);
            console.log(box.label);
            console.log(await delayed(1));
            console.log(await delayed(2));
            console.log(await delayed(3));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_three_way_nullish"),
        "4\nnull\nundefined\n4\n9\n9\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\nnumber\nobject\nundefined\n6\nundefined\nundefined\nnull\n8\nundefined\n11\n11\n12\n5\n0\n0\n8\n0\n0\nnull\nbox\n4\nnull\nundefined\n"
    );
}

#[test]
fn compiles_arrays_of_tagged_nullish_values() {
    let source = r#"
        function optional(present: boolean): number | undefined {
            if (present) return 1;
            return undefined;
        }
        function nullable(present: boolean): number | null {
            if (present) return 2;
            return null;
        }
        function nullish(kind: number): number | null | undefined {
            if (kind === 1) return 3;
            if (kind === 2) return null;
            return undefined;
        }
        function main(): void {
            const optionals: (number | undefined)[] = [
                optional(true), optional(false), optional(true)
            ];
            console.log(optionals[0]);
            console.log(optionals[1]);
            console.log(optionals[2]);
            const nullables: (number | null)[] = [
                nullable(true), nullable(false), nullable(true)
            ];
            console.log(nullables[0]);
            console.log(nullables[1]);
            console.log(nullables[2]);
            const nullishValues: (number | null | undefined)[] = [
                nullish(1), nullish(2), nullish(3)
            ];
            console.log(nullishValues[0]);
            console.log(nullishValues[1]);
            console.log(nullishValues[2]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tagged_nullish_arrays"),
        "1\nundefined\n1\n2\nnull\n2\n3\nnull\nundefined\n"
    );
}

#[test]
fn compiles_number_field_updates_with_single_object_evaluation() {
    let source = r#"
        function pointSource(point: { value: number }): { value: number } {
            console.log("source");
            return point;
        }
        async function asyncPoint(point: { value: number }): Promise<{ value: number }> {
            await sleep(1);
            console.log("async-source");
            return point;
        }
        async function main(): Promise<void> {
            let point = { value: 3 };
            console.log(pointSource(point).value++);
            console.log(point.value);
            console.log(++point.value);
            console.log((await asyncPoint(point)).value--);
            console.log(point.value);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "field_update_expression_values"),
        "source\n3\n4\n5\nasync-source\n5\n4\n"
    );
}

#[test]
fn compiles_dynamic_uniform_object_reads_as_optional_values() {
    let source = r#"
        function makePoint(): { a: number; b: number } {
            console.log("object");
            return { a: 1, b: 2 };
        }
        function selectedKey(): string {
            console.log("key");
            return "b";
        }
        function missingKey(): string { return "missing"; }
        async function asyncKey(): Promise<string> {
            await sleep(1);
            console.log("async-key");
            return "a";
        }
        async function main(): Promise<void> {
            console.log(makePoint()[selectedKey()]);
            const point = { a: 1, b: 2 };
            console.log(point[missingKey()]);
            console.log(point[await asyncKey()]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_computed_properties"),
        "object\nkey\n2\nundefined\nasync-key\n1\n"
    );
}

#[test]
fn deletes_runtime_keyed_dictionary_properties() {
    let source = r#"
        function objectSource(): Record<string, number> {
            console.log("object");
            return { transient: 1 };
        }
        async function asyncKey(): Promise<string> {
            await sleep(1);
            console.log("key");
            return "other";
        }
        async function main(): Promise<void> {
            const values: Record<string, number> = { answer: 42, other: 7 };
            const parsed: Json = JSON.parse("{\"answer\":42}");
            values[1] = 9;
            values[true] = 5;
            console.log(values[1]);
            console.log(values[true]);
            console.log(delete values[1]);
            console.log(delete values.answer);
            console.log(values["answer"]);
            console.log(delete values["missing"]);
            console.log(delete values[await asyncKey()]);
            console.log(values["other"]);
            console.log(delete objectSource().transient);
            console.log(delete parsed.answer);
            console.log(JSON.stringify(parsed));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "delete_dictionary_properties"),
        "9\n5\ntrue\ntrue\n0\ntrue\nkey\ntrue\n0\nobject\ntrue\ntrue\n{}\n"
    );
}

#[test]
fn reads_and_writes_json_with_runtime_string_keys() {
    let source = r#"
        function objectSource(): Json {
            console.log("object");
            return JSON.parse("{\"value\":1}");
        }
        function key(): string {
            console.log("key");
            return "value";
        }
        async function asyncKey(): Promise<string> {
            await sleep(1);
            console.log("async-key");
            return "other";
        }
        async function asyncIndex(): Promise<number> {
            await sleep(1);
            console.log("async-index");
            return 1;
        }
        async function main(): Promise<void> {
            const data: Json = JSON.parse("{\"value\":1,\"1\":\"one\",\"1.5\":\"fraction\"}");
            console.log(String(data[1]));
            console.log(String(data[1.5]));
            data[2] = JSON.parse("\"two\"");
            data[-1] = JSON.parse("\"negative\"");
            console.log(Number(data[key()]));
            data[key()] = JSON.parse("2");
            data.extra = JSON.parse("\"text\"");
            data[await asyncKey()] = JSON.parse("true");
            console.log(Number(data.value));
            console.log(String(data["extra"]));
            console.log(String(data.other));
            console.log(String(objectSource()[key()]));
            console.log(JSON.stringify(data));
            const array: Json = JSON.parse("[10,20]");
            console.log(Number(array[1.5]));
            array[await asyncIndex()] = JSON.parse("30");
            array[3] = JSON.parse("40");
            console.log(String(array[0] = JSON.parse("11")));
            console.log(Number(array[1]));
            console.log(JSON.stringify(array));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "runtime_json_keys"),
        "one\nfraction\nkey\n1\nkey\nasync-key\n2\ntext\ntrue\nobject\nkey\n1\n{\"1\":\"one\",\"2\":\"two\",\"value\":2,\"1.5\":\"fraction\",\"-1\":\"negative\",\"extra\":\"text\",\"other\":true}\n0\nasync-index\n11\n30\n[11,30,null,40]\n"
    );
}

/// `draft.count += 1`-style compound assignment on a `Json`/`any`-typed
/// value used to fail to build ("arithmetic requires F64 operands, got
/// Json and F64") -- an extremely common real-world pattern (any
/// mutation of an `any`/`Json`-typed field or variable, the exact shape
/// `immer`'s standard `draft.count += 1` uses). The compound-assignment
/// lowering only coerced its *right*-hand operand to a number, and only
/// when the *left* side (the property being read back) was already
/// known to be `F64` -- so it silently worked for `n += jsonValue` but
/// not the far more common `jsonValue += n`, where the left side is the
/// one that actually needs converting. Fixed by coercing both operands
/// unconditionally, matching how ordinary (non-compound) arithmetic
/// already does it (`coerce_primitive_to_number` is a no-op for an
/// already-`F64` value, so this isn't a behavior change for the
/// already-working case). Covers every compound arithmetic operator, a
/// `Json`-typed plain variable (not just an object property), and
/// confirms ordinary `F64` compound assignment (a ordinary `let`
/// variable, an array index, a class field) stays unaffected.
#[test]
fn compound_assignment_coerces_a_json_or_any_typed_left_hand_side() {
    let source = r#"
        class Box {
            count: number = 0;
        }
        function main(): void {
            const obj: any = { count: 10 };
            obj.count += 1;
            obj.count -= 3;
            obj.count *= 2;
            obj.count /= 4;
            console.log(obj.count);

            let x: any = 5;
            x += 1;
            console.log(x);

            let y: number = 5;
            y += 1;
            y -= 2;
            y *= 3;
            console.log(y);

            const arr: number[] = [1, 2, 3];
            arr[0] += 10;
            console.log(arr[0]);

            const b = new Box();
            b.count += 5;
            console.log(b.count);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_compound_assignment"),
        "4\n6\n12\n11\n5\n"
    );
}

/// A genuinely missing `Json` key/index used to be indistinguishable
/// from an explicit `null` -- `thaw_json_get`/`thaw_json_index` fell
/// back to plain `Value::Null` either way. Fixed in `thaw-std` (a new
/// `$__thaw_napi_undefined$`-tagged sentinel, matching the one already
/// used for native-callback argument marshaling) and wired through here
/// (`==`/`!=` against `null`/`undefined` used to be a build-time crash,
/// not just a wrong answer -- `lower_loose_equality` had no `Json`-aware
/// arm at all).
#[test]
fn a_missing_json_key_or_index_is_distinguishable_from_an_explicit_null() {
    let source = r#"
        function main(): void {
            const obj: Json = JSON.parse("{\"a\":1,\"n\":null}");
            console.log(obj.n === null);
            console.log(obj.n === undefined);
            console.log(obj.b === null);
            console.log(obj.b === undefined);
            console.log(obj.n == null);
            console.log(obj.n == undefined);
            console.log(obj.b == null);
            console.log(obj.b == undefined);
            console.log(typeof obj.b);
            console.log(String(obj.b));

            const arr: Json = JSON.parse("[1,2]");
            console.log(arr[99] === undefined);
            console.log(arr[99] == null);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_missing_key_vs_null"),
        "true\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\nundefined\nundefined\ntrue\ntrue\n"
    );
}

/// `JSON.stringify` matches real JS's own object-vs-array asymmetry for
/// a nested genuinely `undefined` value: an object field holding one
/// (here, a missing-key-derived value re-inserted elsewhere) is omitted
/// entirely, an array element holding one comes back as `null` -- at any
/// nesting depth, and whether or not a replacer-array/indent argument is
/// also given. Previously left as a deliberate rough edge (`thaw_json_
/// stringify` is also load-bearing for unrelated internal argument/
/// result marshaling that needs the sentinel's raw shape preserved, so
/// it couldn't safely gain this behavior itself) -- fixed by splitting
/// off a `thaw_json_stringify_public` sibling used only by the real,
/// user-facing `JSON.stringify` call, leaving the original function and
/// every internal marshaling path that depends on it untouched.
#[test]
fn json_stringify_omits_or_nulls_a_nested_undefined_value() {
    let source = r#"
        function main(): void {
            const raw: Json = JSON.parse("{\"a\":1}");
            const missing = raw.b;
            const obj: Json = { a: 1, b: missing, c: 3 };
            console.log(JSON.stringify(obj));
            console.log(JSON.stringify(obj, null, 2));
            console.log(JSON.stringify(obj, ["a", "b", "c"]));

            const arr: Json = [1, missing, 3];
            console.log(JSON.stringify(arr));

            console.log(JSON.stringify({ nested: { x: missing, y: 2 } }));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_omits_undefined"),
        "{\"a\":1,\"c\":3}\n{\n  \"a\": 1,\n  \"c\": 3\n}\n{\"a\":1,\"c\":3}\n[1,null,3]\n{\"nested\":{\"y\":2}}\n"
    );
}

/// `JSON.stringify(x)` where `x` *itself* -- the top-level argument, not
/// a nested field/element (see `json_stringify_omits_or_nulls_a_nested_
/// undefined_value` above) -- is genuinely `undefined`. Real JS returns
/// the actual value `undefined` there (`typeof JSON.stringify(x) ===
/// 'undefined'`), which doesn't fit Thaw's own always-a-`Str` calling
/// convention for this call; `console.log` prints the string
/// `"undefined"` instead, matching real JS's own `String(undefined)`
/// coercion for the common case of printing/concatenating the result --
/// a documented, honest approximation (see `top_level_undefined_string`,
/// `thaw-std/src/json.rs`), not a silent miscalculation. Cross-checked
/// against real Node's own `console.log(JSON.stringify(x))` output for
/// exactly this case, which also prints the bare word `undefined`.
#[test]
fn json_stringify_of_a_bare_top_level_undefined_value_prints_the_word_undefined() {
    let source = r#"
        function main(): void {
            const raw: Json = JSON.parse("{\"a\":1}");
            const missing = raw.b;
            console.log(JSON.stringify(missing));
            console.log(JSON.stringify(missing, null, 2));
            console.log(JSON.stringify(missing, ["a"]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_bare_top_level_undefined"),
        "undefined\nundefined\nundefined\n"
    );
}

/// Same top-level-`undefined` case as above, but reached through an
/// `Optional<T>`-typed value (a real `x?: T` parameter) rather than a
/// plain `Json` one -- exercises `wrap_native_value_as_json`'s own path
/// (`JsonSet(..., preserve_undefined: true)`) into the same `thaw_json_
/// stringify_public` fix, not just the direct-`Json` case above.
#[test]
fn json_stringify_of_a_bare_top_level_undefined_optional_value_prints_the_word_undefined() {
    let source = r#"
        function stringifyIt(x?: number): string {
            return JSON.stringify(x);
        }
        function main(): void {
            console.log(stringifyIt(undefined));
            console.log(stringifyIt(42));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_bare_top_level_undefined_optional"),
        "undefined\n42\n"
    );
}

/// The literal, purely-static form of the same case:
/// `JSON.stringify(undefined)` with the bare `undefined` keyword itself
/// as the argument (`HirType::Undefined`, previously rejected at build
/// time entirely -- `json_convertible_native_type` didn't list it).
/// `wrap_native_value_as_json`'s temporary-object round trip already
/// handles this correctly for free: `JsonSet` treats a bare
/// `HirType::Undefined` field as a complete no-op (the field is never
/// set at all, matching an ordinary object literal's own "legitimately
/// omitted field" convention), so reading it straight back out finds a
/// genuinely *missing* key -- which this crate's own JSON layer already
/// represents as the same napi-undefined sentinel a real missing key
/// produces, landing on the identical fix above with no new codegen.
#[test]
fn json_stringify_of_the_literal_undefined_keyword_prints_the_word_undefined() {
    let source = r#"
        function main(): void {
            console.log(JSON.stringify(undefined));
            console.log(JSON.stringify(undefined, null, 2));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_literal_undefined"),
        "undefined\nundefined\n"
    );
}

/// `new <Namespace>.<Class>(...)` -- a namespaced global constructor
/// (real example: `new Intl.DateTimeFormat(...)`, needed for the `Intl`
/// polyfill) used to fail to compile at all ("only `new Promise<T>(...)`
/// is supported"): thaw-hir's special-cased list of constructible
/// globals (`TextEncoder`/`AbortController`/typed arrays/...) only ever
/// matched a bare `Expr::Ident` callee, never `Expr::Member` -- so a
/// namespaced one fell straight through to the Promise-only fallback.
/// Fixed by reusing the exact same `getDynamicValue`/
/// `constructDynamicValue` mechanism those already use, just with a
/// dotted name (`"Intl.DateTimeFormat"`); `thaw_js_get_global`
/// (`crates/thaw-quickjs/src/quickjs/api.rs`) now walks nested object
/// properties for a dotted path instead of only a single flat global
/// lookup.
#[test]
fn constructs_a_namespaced_global_value_via_new() {
    let source = r#"
        function main(): void {
            const dtf: JsValue = new Intl.DateTimeFormat("en-US", {
                year: "numeric", month: "2-digit", day: "2-digit", timeZone: "UTC"
            } as JsValue);
            console.log(dtf.format(new Date("2024-07-04T16:30:45.000Z")));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "constructs_namespaced_global"),
        "07/04/2024\n"
    );
}

/// A relational comparison (`<`/`>`/etc, `lower_relational`) against a
/// statically `null`/`undefined`-typed operand used to crash at build
/// time -- `coerce_primitive_to_number` had no arm for either type,
/// only an `Err("numeric conversion is not defined for native type
/// ...")` catch-all. Fixed by adding real JS's own `ToNumber` behavior
/// directly (`Number(null) === 0`, `Number(undefined) === NaN` -- these
/// differ from each other, so a shared fallback can't collapse them);
/// found complementary to (but independent of) the `Json`/`JsValue`
/// loose-equality fix, which only reaches this function for *other*
/// type combinations. `alwaysNull()`'s side effect (the `console.log`)
/// is still evaluated even though its coerced value is a compile-time
/// constant.
#[test]
fn a_relational_comparison_against_null_or_undefined_coerces_instead_of_crashing() {
    let source = r#"
        function alwaysNull(): null {
            console.log("called");
            return null;
        }
        function alwaysUndefined(): undefined {
            return undefined;
        }
        function main(): void {
            console.log(5 < alwaysNull());
            console.log(5 < alwaysUndefined());
            console.log(alwaysNull() < 5);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "relational_null_undefined_coercion"),
        "called\nfalse\nfalse\ncalled\ntrue\n"
    );
}

#[test]
fn stringifies_json_with_number_and_string_spacing() {
    let source = r#"
        function value(): Json {
            console.log("value");
            return JSON.parse("{\"b\":2,\"a\":{\"x\":1}}");
        }
        async function spacing(): Promise<number> {
            await sleep(1);
            console.log("space");
            return 2.9;
        }
        async function keys(): Promise<string[]> {
            await sleep(1);
            console.log("replacer");
            return ["a", "x", "a"];
        }
        async function main(): Promise<void> {
            console.log(JSON.stringify(value(), null, await spacing()));
            console.log(JSON.stringify(JSON.parse("[1]"), undefined, "--"));
            console.log(JSON.stringify(JSON.parse("{\"x\":1}"), null, "abcdefghijkl"));
            console.log(JSON.stringify(JSON.parse("[1]"), null, 20));
            console.log(JSON.stringify(JSON.parse("{\"x\":1}"), null, -1));
            console.log(JSON.stringify(
                JSON.parse("{\"a\":{\"x\":1,\"y\":2},\"b\":3}"),
                ["b", "a", "x", "b"]
            ));
            console.log(JSON.stringify(value(), await keys(), "--"));
            console.log(JSON.stringify(JSON.parse("[{\"a\":1,\"b\":2}]"), ["a"]));
            const record: Record<string, number> = { answer: 42 };
            console.log(JSON.stringify(record));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_spacing"),
        "value\nspace\n{\n  \"b\": 2,\n  \"a\": {\n    \"x\": 1\n  }\n}\n[\n--1\n]\n{\nabcdefghij\"x\": 1\n}\n[\n          1\n]\n{\"x\":1}\n{\"b\":3,\"a\":{\"x\":1}}\nvalue\nreplacer\n{\n--\"a\": {\n----\"x\": 1\n--}\n}\n[{\"a\":1}]\n{\"answer\":42}\n"
    );
}

#[test]
fn stringifies_native_typed_values() {
    let source = r#"
        interface Point { x: number; y: number; }
        interface Shape { name: string; points: Point[]; }
        async function main(): Promise<void> {
            const shape: Shape = {
                name: "triangle",
                points: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 0, y: 1 }],
            };
            console.log(JSON.stringify(shape));
            const tuple: [number, string] = [42, "hi"];
            console.log(JSON.stringify(tuple));
            console.log(JSON.stringify([1, 2, 3]));
            console.log(JSON.stringify(42));
            console.log(JSON.stringify("hi"));
            console.log(JSON.stringify(true));
            console.log(JSON.stringify(shape, null, 2));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "json_stringify_native_values"),
        concat!(
            "{\"name\":\"triangle\",\"points\":[{\"x\":0,\"y\":0},{\"x\":1,\"y\":0},{\"x\":0,\"y\":1}]}\n",
            "[42,\"hi\"]\n",
            "[1,2,3]\n",
            "42\n",
            "\"hi\"\n",
            "true\n",
            "{\n  \"name\": \"triangle\",\n  \"points\": [\n    {\n      \"x\": 0,\n      \"y\": 0\n    },\n    {\n      \"x\": 1,\n      \"y\": 0\n    },\n    {\n      \"x\": 0,\n      \"y\": 1\n    }\n  ]\n}\n",
        )
    );
}

#[test]
fn compiles_structured_clone_of_native_values() {
    let source = r#"
        interface Point { x: number; y: number; }
        interface Shape { name: string; points: Point[]; }
        async function main(): Promise<void> {
            const original: Point = { x: 1, y: 2 };
            const clone = structuredClone(original);
            clone.x = 999;
            console.log(original.x, clone.x);

            const arr = [1, 2, 3];
            const arrClone = structuredClone(arr);
            arrClone[0] = 100;
            console.log(arr[0], arrClone[0]);

            console.log(structuredClone(42));
            console.log(structuredClone("hi"));
            console.log(structuredClone(true));

            const shape: Shape = { name: "s", points: [{ x: 0, y: 0 }, { x: 1, y: 1 }] };
            const shapeClone = structuredClone(shape);
            shapeClone.points[0].x = 500;
            console.log(shape.points[0].x, shapeClone.points[0].x);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "structured_clone_native_values"),
        "1 999\n1 100\n42\nhi\ntrue\n0 500\n"
    );
}

#[test]
fn structured_clone_consumes_union_arrays_once_and_keeps_clones_independent() {
    let source = r#"
        interface NumberBox { value: number; }
        interface TextBox { value: string; }
        let calls = 0;
        function boxes(numbers: boolean): NumberBox[] | TextBox[] {
            calls++;
            if (numbers) return [{ value: 1 }, , { value: 3 }];
            return [{ value: "a" }, , { value: "c" }];
        }
        function check(numbers: boolean): void {
            const original = boxes(numbers);
            const clone = structuredClone(original);
            clone.pop();
            console.log(String(original[0].value), String(clone[0].value));
            console.log(original.length, clone.length, 1 in clone);
        }
        function main(): void {
            check(true);
            check(false);
            console.log(calls);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "structured_clone_union_arrays"),
        "1 1\n3 2 false\na a\n3 2 false\n2\n"
    );
}

#[test]
fn compiles_structured_clone_of_a_map_or_set() {
    // Builds a fresh Map/Set and recursively clones each entry, snapshotting
    // the source via the same conversion `[...map]`/`Array.from(map)` use.
    // The nested-array case checks that cloning is real and deep (not a
    // shared reference) at every level: the receiver Map, and each array
    // stored as one of its values.
    let source = r#"
        async function main(): Promise<void> {
            const originalSet = new Set<number>([1, 2, 3]);
            const clonedSet = structuredClone(originalSet);
            clonedSet.add(4);
            console.log(originalSet.size, clonedSet.size);

            const originalMap = new Map<string, number[]>([
                ["a", [1, 2]],
                ["b", [3]],
            ]);
            const clonedMap = structuredClone(originalMap);
            const clonedBucket = clonedMap.get("a");
            if (clonedBucket !== undefined) {
                clonedBucket.push(99);
                console.log(clonedBucket.join(","));
            }
            const originalBucket = originalMap.get("a");
            if (originalBucket !== undefined) {
                console.log(originalBucket.join(","));
            }
            console.log(clonedMap.size);

            const emptyMap = structuredClone(new Map<string, number>());
            console.log(emptyMap.size);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "structured_clone_map_or_set"),
        "3 4\n1,2,99\n1,2\n2\n0\n"
    );
}

/// `structuredClone` on a dynamic (`any`/`Json`-typed) value -- the common
/// case for loosely-typed data, as opposed to the fixed-shape native
/// `Object`/`Array` case above. `Value`'s derived `Clone` is already a
/// full recursive deep copy, so mutating a top-level field of the clone
/// (the compiler's existing support for writing an `any`-typed field,
/// independent of this fix) never touches the original.
#[test]
fn compiles_structured_clone_of_a_dynamic_value() {
    let source = r#"
        async function main(): Promise<void> {
            const original: any = { a: 1, b: "x", nested: { d: true } };
            const clone = structuredClone(original);
            clone.a = 999;
            console.log(JSON.stringify(original));
            console.log(JSON.stringify(clone));
            console.log(original === clone);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "structured_clone_dynamic_value"),
        "{\"a\":1,\"b\":\"x\",\"nested\":{\"d\":true}}\n{\"a\":999,\"b\":\"x\",\"nested\":{\"d\":true}}\nfalse\n"
    );
}

/// Nested writes through an `any`-typed value (`a.b.c = x`, `a.b[i] = x`)
/// used to silently no-op: reading `a.b` as an assignment target's
/// intermediate container went through the same clone-returning read as
/// any other `.field` expression (`thaw_json_get`), so the write landed
/// on a disconnected copy instead of `a`'s own nested storage -- pinned
/// against real Node output. Also covers auto-vivifying a missing
/// intermediate field (real JS would throw; this compiler has no
/// exception channel on this bridge yet, so it degrades to creating the
/// object instead, per this file's own module doc comment) and a
/// 3-level chain ending in a compound assignment.
#[test]
fn compiles_nested_dynamic_value_assignment() {
    let source = r#"
        async function main(): Promise<void> {
            const obj: any = { c: { d: true } };
            obj.c.d = false;
            console.log(JSON.stringify(obj));

            const arr: any = { c: [1, 2, 3] };
            arr.c[0] = 99;
            console.log(JSON.stringify(arr));

            const deep: any = { p: { q: { r: 5 } } };
            deep.p.q.r *= 3;
            console.log(JSON.stringify(deep));

            const fresh: any = {};
            fresh.a.b = 5;
            console.log(JSON.stringify(fresh));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "nested_dynamic_value_assignment"),
        "{\"c\":{\"d\":false}}\n{\"c\":[99,2,3]}\n{\"p\":{\"q\":{\"r\":15}}}\n{\"a\":{\"b\":5}}\n"
    );
}

#[test]
fn json_stringify_calls_function_replacers() {
    let source = r#"function main(): void {
        const value: Json = JSON.parse('{"keep":2,"other":3}');
        console.log(JSON.stringify(value, (key: string, current: Json): Json => {
            return key === "keep" ? JSON.parse("4") : current;
        }));
        console.log(JSON.stringify(
            value,
            (key: string, current: Json): Json => current,
            2
        ));
        console.log(JSON.stringify(value, (key: string, current: Json): any => {
            return key === "other" ? undefined : current;
        }));
    }"#;
    assert_eq!(
        compile_and_run(source, "json_stringify_function_replacer"),
        "{\"keep\":4,\"other\":3}\n{\n  \"keep\": 2,\n  \"other\": 3\n}\n{\"keep\":2}\n"
    );
}

/// `===` between two dynamic (`Json`-typed) values used to always compare
/// unequal for a primitive (`const a: any = 1; const b: any = 1; a === b`
/// was `false`): both are separate heap allocations, and the native
/// `===` codegen otherwise falls back to a raw pointer compare for any
/// non-string pointer-shaped operand. Real Strict Equality Comparison
/// compares a number/string/boolean by value; an array/object still
/// compares by reference (`{} === {}` stays `false`).
///
/// Doesn't cover `NaN` assigned through `any`: `number_value` (thaw-std)
/// falls back to `Value::Null` for any non-finite `f64` (`serde_json::
/// Number` structurally can't hold one), so a `Json`-typed `NaN` is
/// already indistinguishable from real `null` well before this equality
/// fix -- confirmed pre-existing and separate (`typeof` on it already
/// said `"object"`, `=== null` was already `true`). Left as a documented,
/// separate, un-fixed gap; asserting today's `NaN === NaN` behavior here
/// would enshrine that existing bug as if it were this fix's contract.
#[test]
fn compiles_dynamic_value_strict_equality() {
    let source = r#"
        function main(): void {
            const a: any = 1;
            const b: any = 1;
            console.log(a === b);
            const c: any = "x";
            const d: any = "x";
            console.log(c === d);
            const e: any = true;
            const f: any = true;
            console.log(e === f);
            const h: any = {};
            const i: any = {};
            console.log(h === i);
            console.log(h === h);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_value_strict_equality"),
        "true\ntrue\ntrue\nfalse\ntrue\n"
    );
}

#[test]
fn compares_present_optional_strings_by_value() {
    let source = r#"
        function main(): void {
            const values: string[] = ["same"];
            const empty: string[] = [];
            console.log(values[0] === "same");
            console.log("same" === values[0]);
            console.log(empty[0] === "same");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "optional_string_strict_equality"),
        "true\ntrue\nfalse\n"
    );
}

/// `NaN`/`Infinity`/`-Infinity` boxed into an `any`-typed (`Json`)
/// value used to be indistinguishable from a real `null`:
/// `serde_json::Number` structurally cannot hold a non-finite `f64`, so
/// `number_value` (thaw-std's `json.rs`) silently fell back to
/// `Value::Null`. `x === null` was `true`, `typeof x` was `"object"`,
/// `Number.isNaN(x)` was `false`, and `.includes()` couldn't find a
/// `NaN` needle -- all wrong. Every one of these consumers
/// (`thaw_json_typeof`, `json_to_number`, `thaw_json_strict_equal`/
/// `json_same_value_zero`) was already built to recognize a
/// `{"$__thaw_non_finite$": "NaN"|"Infinity"|"-Infinity"}` sentinel
/// object (the same shape `platform_globals/dates.js`'s
/// `__thaw_json_safe_stringify` JS-side replacer already produces for
/// values crossing the QuickJS boundary) -- `number_value` just never
/// produced it for the far more common native-compiled-code path.
/// Fixed there, plus made the `JSON.stringify`/`console.log`/`String()`
/// families recognize the sentinel too, matching real JS's own quirk
/// that `JSON.stringify` collapses a non-finite number to `null` (kept)
/// while every other consumer must see the real value.
///
/// Also closed in the same pass: `.includes()` on an `any[]` used
/// `thaw_json_strict_equal` (`===`, `NaN` never equals itself) for
/// searching, same as `.indexOf()`/`.lastIndexOf()` -- correct for
/// those, but `.includes()`'s own spec uses `SameValueZero` (`NaN`
/// *does* find itself). Undetectable before this fix, since a `NaN`
/// needle and a `NaN` haystack element were both just `Value::Null`,
/// so `===` "found" it by accident either way.
#[test]
fn compiles_dynamic_non_finite_number_identity() {
    let source = r#"
        function main(): void {
            const nan: any = NaN;
            const inf: any = Infinity;
            const ninf: any = -Infinity;
            console.log(nan === null, nan === nan, typeof nan);
            console.log(inf === null, typeof inf, inf === Infinity);
            console.log(Number.isNaN(nan), Number.isNaN(inf), Number.isNaN(1));
            console.log(Number.isFinite(nan), Number.isFinite(inf), Number.isFinite(1));
            console.log(String(nan), String(inf), String(ninf));
            console.log(JSON.stringify(nan), JSON.stringify({ a: nan }));
            const arr: any[] = [1, nan, inf, ninf, 5];
            console.log(arr.indexOf(NaN), arr.includes(NaN));
            console.log(JSON.stringify(arr));
            const m = new Map<any, string>();
            m.set(NaN, "nanvalue");
            console.log(m.get(NaN), m.has(NaN));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_non_finite_number_identity"),
        concat!(
            "false false number\n",
            "false number true\n",
            "true false false\n",
            "false false true\n",
            "NaN Infinity -Infinity\n",
            "null {\"a\":null}\n",
            "-1 true\n",
            "[1,null,null,null,5]\n",
            "nanvalue true\n",
        )
    );
}

/// `Object.keys`/`Object.values`/`Object.entries` (and `Reflect.
/// ownKeys`/the typed-`Dictionary` variants) on a `Json` value were
/// never registered in `expr_hir_type` (thaw-llvm's own mirror of
/// thaw-hir's `infer_expr_type`, which *does* have them) -- harmless
/// when the result is bound to a `let`/`const` first, but
/// `console.log(Object.keys(x))` used directly inline hit `expr_hir_
/// type` returning `None` for that argument, and `compile_console_arg`'s
/// `None` fallback treated the resulting array handle as an *error*
/// value to report via `thaw_error_message` instead of formatting it as
/// an array -- observed printing garbage/replacement-character bytes.
#[test]
fn compiles_dynamic_object_introspection_used_inline_in_console_log() {
    let source = r#"
        function main(): void {
            const obj: any = { a: 1, b: 2 };
            console.log(Object.keys(obj));
            console.log(Object.values(obj));
            console.log(Object.entries(obj));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_object_introspection_inline_console_log"),
        "[\"a\",\"b\"]\n[1,2]\n[[\"a\",1],[\"b\",2]]\n"
    );
}

/// `{...anyValue, key: value}` (an object literal spreading a
/// dynamically-shaped source -- `Json` or `Dictionary`, not a
/// compile-time-known `HirType::Object`) used to hard-error ("cannot
/// spread a value of type Json into an object literal"):
/// `lower_object_lit`'s normal path merges a spread directly into a
/// static `HirExpr::ObjectLit`'s own field list, which needs a
/// compile-time field set to expand. Fixed with a separate runtime path
/// (`lower_dynamic_spread_object_lit`, thaw-hir's `objects.rs`): start
/// from an empty `Json` object and walk the literal's properties *in
/// source order*, `__thaw_json_object_assign`-merging a spread's keys
/// wholesale or `JsonSet`-ing a single key -- matching real JS's own
/// "a later spread/key overwrites an earlier one's value without moving
/// its position" semantics. Covers override order both directions, a
/// spread-only literal, multiple spreads, and mixing a genuinely dynamic
/// spread with a fixed-shape `Object`/`Dictionary` one in the same
/// literal (the pre-existing path, confirmed unaffected).
///
/// Found and fixed along with this: `thaw_json_object_assign` (backing
/// both `Object.assign` and this new spread path) copied a `null`/
/// `undefined` source's fields verbatim instead of skipping it (real JS:
/// both are silently no-ops, never an error) -- for `undefined`
/// specifically this was a real, severe **data-corruption** bug, not
/// just a missing no-op: the napi-undefined sentinel value is itself
/// structurally a `Value::Object` with one internal tracking key
/// (`$__thaw_napi_undefined$`), so merging it copied that key straight
/// into the target, making every *later* `is_napi_undefined` check
/// (`typeof`, `JSON.stringify`, ...) see the **entire merged object** as
/// itself being `undefined` -- `{...maybeUndefined, b: 2}` silently lost
/// every field, not just the nullish source's own absence of any. Also
/// fixed the same function to copy an array source's own enumerable
/// properties (index keys, not `length`) rather than nothing, matching
/// `Object.assign({}, [1,2,3])` -> `{0:1,1:2,2:3}`.
#[test]
fn compiles_dynamic_object_literal_spread() {
    let source = r#"
        interface Fixed { x: number; y: number; }
        let order = "";
        function computedKey(label: string, key: string): string {
            order += label;
            return key;
        }
        function computedValue(label: string, value: number): number {
            order += label;
            return value;
        }
        function main(): void {
            const obj: any = { a: 1, b: "x", c: true };
            console.log(JSON.stringify({ ...obj, d: 4 }));
            console.log(JSON.stringify({ ...obj, a: 99 }));
            console.log(JSON.stringify({ a: 99, ...obj }));
            console.log(JSON.stringify({ ...obj }));
            console.log(JSON.stringify({ ...obj, ...{ e: 5, a: 100 } }));
            const fixed: Fixed = { x: 1, y: 2 };
            console.log(JSON.stringify({ ...fixed, ...obj, z: 3 }));
            const dict: Record<string, number> = { p: 10, q: 20 };
            console.log(JSON.stringify({ ...dict, r: 30 }));
            const arr: any = [1, 2, 3];
            console.log(JSON.stringify({ ...arr }));
            const nullish: any = null;
            const missing: any = undefined;
            console.log(JSON.stringify({ a: 1, ...nullish, ...missing, b: 2 }));
            const target: any = { a: 1 };
            Object.assign(target, nullish, missing, { b: 2 });
            console.log(JSON.stringify(target));
            const computed = {
                [computedKey("k1", "b")]: computedValue("v1", 20),
                ...obj,
                [computedKey("k2", "d")]: computedValue("v2", 4),
                ...{ b: 5 },
                [computedKey("k3", "a")]: computedValue("v3", 9),
            };
            console.log(order);
            console.log(JSON.stringify(computed));
            const numericKey = 2;
            console.log(JSON.stringify({ ...obj, [numericKey]: 8 }));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_object_literal_spread"),
        concat!(
            "{\"a\":1,\"b\":\"x\",\"c\":true,\"d\":4}\n",
            "{\"a\":99,\"b\":\"x\",\"c\":true}\n",
            "{\"a\":1,\"b\":\"x\",\"c\":true}\n",
            "{\"a\":1,\"b\":\"x\",\"c\":true}\n",
            "{\"a\":100,\"b\":\"x\",\"c\":true,\"e\":5}\n",
            "{\"x\":1,\"y\":2,\"a\":1,\"b\":\"x\",\"c\":true,\"z\":3}\n",
            "{\"p\":10,\"q\":20,\"r\":30}\n",
            "{\"0\":1,\"1\":2,\"2\":3}\n",
            "{\"a\":1,\"b\":2}\n",
            "{\"a\":1,\"b\":2}\n",
            "k1v1k2v2k3v3\n",
            "{\"b\":5,\"a\":9,\"c\":true,\"d\":4}\n",
            "{\"2\":8,\"a\":1,\"b\":\"x\",\"c\":true}\n",
        )
    );
}

/// `.includes()`/`.indexOf()`/`.lastIndexOf()` on an `any[]` (`Json`
/// element) array used to either silently return `false`/`-1` (when the
/// needle's own natural type, e.g. `number`, differed from the array's
/// declared `any` element type) or hit a hard compile error ("array
/// search does not support element type Json", when the needle was
/// already `any`-typed) -- the element-type dispatch this shares with
/// `number[]`/`string[]`/etc. had no `Json` case at all. Comparison is
/// real Strict Equality (`thaw_json_strict_equal`, same as `===` above),
/// not pointer identity, since a `Json` slot can hold a primitive.
#[test]
fn compiles_dynamic_array_search_methods() {
    let source = r#"
        function main(): void {
            const a: any[] = [1, "two", true, 1, null];
            console.log(a.includes(1));
            console.log(a.includes(2));
            console.log(a.indexOf(1));
            console.log(a.lastIndexOf(1));
            console.log(a.indexOf("two"));
            console.log(a.includes(null));
            console.log(a.includes(undefined));
            const needle: any = 1;
            console.log(a.includes(needle));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_array_search_methods"),
        "true\nfalse\n0\n3\n1\ntrue\nfalse\ntrue\n"
    );
}

/// `switch` on an `any`-typed discriminant used to reject any `case`
/// value outright ("switch case has type F64, expected Json") -- each
/// `case` test only got a strict type *check* against the declared `any`
/// (`Json`) discriminant, not the same declared-type coercion an
/// ordinary call argument gets. Matching itself already went through
/// `===`'s codegen (`BinOp::EqEqEq`), so this reuses the same Strict
/// Equality fix above once the type mismatch stopped blocking it outright.
#[test]
fn compiles_dynamic_switch_statement() {
    let source = r#"
        function describe(x: any): string {
            switch (x) {
                case 1: return "one";
                case 2: return "two";
                default: return "other";
            }
        }
        function main(): void {
            console.log(describe(1));
            console.log(describe(2));
            console.log(describe(3));
            const y: any = "b";
            switch (y) {
                case "a": console.log("a"); break;
                case "b": console.log("b"); break;
                default: console.log("?");
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_switch_statement"),
        "one\ntwo\nother\nb\n"
    );
}

/// `.sort()`/`.toSorted()` with no comparator on an `any[]` array used to
/// hit a hard compile error ("default array sort does not support
/// element type Json") -- the element-type dispatch had no `Json` case.
/// Real default sort converts each element to its `String()` form and
/// compares lexicographically; this already exists as a generic,
/// element-type-agnostic comparator (`lower_array_sort_default_tagged`,
/// originally built for `Optional`/`Nullable`/etc. array elements), so
/// `Json` just needed routing through it too.
#[test]
fn compiles_dynamic_default_array_sort() {
    let source = r#"
        function main(): void {
            const a: any[] = [3, 1, 20, "b", "a"];
            console.log(a.sort().join(","));
            const b: any[] = [3, 1, 2];
            console.log(b.toSorted().join(","));
            console.log(b.join(","));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_default_array_sort"),
        "1,20,3,a,b\n1,2,3\n3,1,2\n"
    );
}

/// A plain literal following a `...spread` of an `any[]` array in an
/// array literal (`[...anyArr, "x", true, 0]`) used to hit a hard
/// compile error ("array element type F64 does not match Json") -- once
/// the spread source fixed the literal's own inferred element type to
/// `Json`, each subsequent plain element only got a strict type check
/// against it, not the same declared-type coercion an ordinary element
/// already gets.
#[test]
fn compiles_array_literal_spreads_any_then_mixed_literals() {
    let source = r#"
        function main(): void {
            const arrSpread: any[] = [1, 2];
            console.log(JSON.stringify([...arrSpread, "x", true, 0]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_literal_spreads_any_then_mixed_literals"),
        "[1,2,\"x\",true,0]\n"
    );
}

/// A `Union`-typed value (e.g. a ternary/`if`-`else` naturally inferring a
/// union of its branch types) assigned to an `any`-typed slot used to hit
/// a hard compile error ("value has type Union([...]), expected Json") --
/// `json_convertible_native_type`'s own list of native types the generic
/// `Json` coercion knows how to encode was missing `Union`, even though
/// the actual encoder it gates (`compile_json_object_set_union`,
/// thaw-llvm) already recursively re-dispatches to whichever member is
/// actually tagged active, so no new codegen was needed, just the gate.
#[test]
fn compiles_union_value_coerced_to_dynamic() {
    let source = r#"
        function returnsEither(flag: boolean): any {
            return flag ? 1 : "x";
        }
        function main(): void {
            const cond: any = true ? 1 : "str";
            console.log(cond);
            console.log(returnsEither(true), returnsEither(false));
            const tuple: [any, any] = [1, "x"];
            console.log(tuple[0], tuple[1]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "union_value_coerced_to_dynamic"),
        "1\n1 x\n1 x\n"
    );
}

/// `x ?? y` on a dynamic (`any`-typed) `x` used to silently keep `x`
/// even when it was `null`/`undefined` at runtime -- unlike `Optional`/
/// `Nullable`/`Nullish`, whose absent state is a compile-time-known tag,
/// a `Json` value's nullish-ness is only knowable at runtime
/// (`__thaw_json_is_nullish`, the same check `==`/`===` against a bare
/// `null`/`undefined` literal already uses), and this fell through to
/// the lowering's generic "not a taggable type" fallback instead. A
/// falsy-but-not-nullish value (`0`, `""`) still correctly keeps `x`,
/// same as real `??` (unlike `||`).
#[test]
fn compiles_dynamic_nullish_coalescing() {
    let source = r#"
        function main(): void {
            const m: any = null;
            console.log(m ?? "default");
            const u: any = undefined;
            console.log(u ?? "default2");
            const v: any = 0;
            console.log(v ?? "default3");
            const s: any = "";
            console.log(s ?? "default4");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_nullish_coalescing"),
        "default\ndefault2\n0\n\n"
    );
}

/// `x ??= y` on a dynamic (`any`-typed) variable/property target used to
/// silently never assign, even when `x` was really `null`/`undefined`
/// (same root cause as plain `??` above: only `Optional`/`Nullable`/
/// `Nullish` were recognized as possibly-nullish, and `Json`'s nullish-
/// ness is only knowable at runtime). Also confirms real `??=`'s own
/// short-circuit: the RHS (with a side effect) is never evaluated when
/// the target isn't nullish.
#[test]
fn compiles_dynamic_nullish_assignment() {
    let source = r#"
        function main(): void {
            let m: any = null;
            m ??= "default";
            console.log(m);
            let u: any = undefined;
            u ??= "default2";
            console.log(u);
            const obj: { v: any } = { v: null };
            obj.v ??= "prop-default";
            console.log(obj.v);
            let side = 0;
            let already: any = 5;
            already ??= (side = 1, "unused");
            console.log(already, side);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_nullish_assignment"),
        "default\ndefault2\nprop-default\n5 0\n"
    );
}

#[test]
fn compiles_dynamic_heterogeneous_object_reads_as_unions() {
    let source = r#"
        function read(key: string): number | string | null | undefined {
            const mixed = {
                value: 41, label: "thaw", empty: null, absent: undefined
            };
            return mixed[key];
        }
        function readTagged(key: string): number | string | null | undefined {
            const mixed: {
                optionalValue: number | undefined;
                optionalAbsent: number | undefined;
                nullableValue: number | null;
                nullableNull: number | null;
                nullishValue: number | null | undefined;
                nullishNull: number | null | undefined;
                nullishUndefined: number | null | undefined;
                label: string;
            } = {
                optionalValue: 6,
                optionalAbsent: undefined,
                nullableValue: 7,
                nullableNull: null,
                nullishValue: 8,
                nullishNull: null,
                nullishUndefined: undefined,
                label: "tagged"
            };
            return mixed[key];
        }
        function print(key: string): void {
            const value = read(key);
            if (typeof value === "number") {
                console.log(value + 1);
            } else if (typeof value === "string") {
                console.log(value + "!");
            } else if (typeof value === "object") {
                console.log(value === null);
            } else {
                console.log(value);
            }
        }
        function main(): void {
            print("value");
            print("label");
            print("empty");
            print("absent");
            print("missing");
            console.log(readTagged("optionalValue"));
            console.log(readTagged("optionalAbsent"));
            console.log(readTagged("nullableValue"));
            console.log(readTagged("nullableNull"));
            console.log(readTagged("nullishValue"));
            console.log(readTagged("nullishNull"));
            console.log(readTagged("nullishUndefined"));
            console.log(readTagged("label"));
            console.log(readTagged("missing"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_heterogeneous_properties"),
        "42\nthaw!\ntrue\nundefined\nundefined\n6\nundefined\n7\nnull\n8\nnull\nundefined\ntagged\nundefined\n"
    );
}

#[test]
fn compiles_dynamic_uniform_tagged_object_reads_without_nested_tags() {
    let source = r#"
        function key(value: string): string { return value; }
        async function delayedKey(): Promise<string> {
            await sleep(1);
            return "nullValue";
        }
        async function main(): Promise<void> {
            const optional: {
                value: number | undefined;
                absent: number | undefined;
            } = { value: 1, absent: undefined };
            console.log(optional[key("value")]);
            console.log(optional[key("absent")]);
            console.log(optional[key("missing")]);

            const nullable: {
                value: number | null;
                nullValue: number | null;
            } = { value: 2, nullValue: null };
            console.log(nullable[key("value")]);
            console.log(nullable[await delayedKey()]);
            console.log(nullable[key("missing")]);

            const nullish: {
                value: number | null | undefined;
                nullValue: number | null | undefined;
                absent: number | null | undefined;
            } = { value: 3, nullValue: null, absent: undefined };
            console.log(nullish[key("value")]);
            console.log(nullish[key("nullValue")]);
            console.log(nullish[key("absent")]);
            console.log(nullish[key("missing")]);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "dynamic_tagged_computed_properties"),
        "1\nundefined\nundefined\n2\nnull\nundefined\n3\nnull\nundefined\nundefined\n"
    );
}
