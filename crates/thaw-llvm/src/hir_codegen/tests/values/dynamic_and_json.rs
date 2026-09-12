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

#[test]
fn rejects_json_stringify_function_replacers() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const value: Json = JSON.parse("{}");
            console.log(JSON.stringify(
                value,
                (key: string, current: Json): Json => current
            ));
        }"#,
    )
    .unwrap();
    let error = thaw_hir::lower_module(&module).unwrap_err();
    assert!(error.contains("function replacers"), "{error}");
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

