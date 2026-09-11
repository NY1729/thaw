/// A registry Fallback function's overloads discriminated by *arity
/// range* -- real example: uuid's `v4(options?): string` (0-1 args)
/// alongside its generic buffer-output `v4<TBuf extends Uint8Array =
/// Uint8Array>(options, buf, offset?): TBuf` (2-3 args), previously
/// unreachable no matter what a real call passed since "first
/// successful overload wins" only ever exposed the first. Confirms
/// `rewrite_fallback_function_overloads` (a bare-call analog of
/// `rewrite_external_class_methods`) picks the right helper purely by
/// how many arguments a call site actually passes.
#[test]
fn selects_fallback_function_overloads_by_arity_range() {
    let source = "makeId(); makeId(1); makeId(1, 2);";
    let rewritten = rewrite_fallback_function_overloads(
        source,
        &[
            (
                "makeId".into(),
                "__makeId_default".into(),
                0,
                1,
                vec![thaw_hir::HirType::F64],
                None,
            ),
            (
                "makeId".into(),
                "__makeId_buffer".into(),
                2,
                2,
                vec![thaw_hir::HirType::F64, thaw_hir::HirType::F64],
                None,
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "__makeId_default(); __makeId_default(1); __makeId_buffer(1, 2);"
    );
}

/// A registry Fallback function's overloads with *identical* arity,
/// discriminated only by each argument's actual type -- the same
/// disambiguation `selects_same_arity_external_method_overloads_by_
/// argument_type` already confirms for class methods, applied to a bare
/// function call instead.
#[test]
fn selects_fallback_function_overloads_with_the_same_arity_by_argument_type() {
    let source =
        "const n = 42; const s = \"hello\"; describe(n); describe(s); describe(7); describe(\"world\");";
    let rewritten = rewrite_fallback_function_overloads(
        source,
        &[
            (
                "describe".into(),
                "__describe_number".into(),
                1,
                1,
                vec![thaw_hir::HirType::F64],
                None,
            ),
            (
                "describe".into(),
                "__describe_string".into(),
                1,
                1,
                vec![thaw_hir::HirType::Str],
                None,
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const n = 42; const s = \"hello\"; __describe_number(n); __describe_string(s); __describe_number(7); __describe_string(\"world\");"
    );
}

#[test]
fn selects_same_arity_external_method_overloads_by_argument_type() {
    let source = "const box = new NativeBox(1); const n = 42; const s = \"hello\"; box.set(n); box.set(s); box.set(7); box.set(\"world\");";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = NativeBox_ctor(1); const n = 42; const s = \"hello\"; __set_number(box, n); __set_string(box, s); __set_number(box, 7); __set_string(box, \"world\");"
        );
}

#[test]
fn selects_external_overloads_from_annotations_and_assertions() {
    let source = r#"const box = new NativeBox(1); let declared: string; const asserted = 42 as string; const angle = <number>unknown; box.set(declared); box.set(asserted); box.set(angle); box.set(unknown as boolean);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_string(box, declared)"));
    assert!(rewritten.contains("__set_string(box, asserted)"));
    assert!(rewritten.contains("__set_number(box, angle)"));
    assert!(rewritten.contains("__set_boolean(box, unknown as boolean)"));
}

#[test]
fn selects_number_array_overloads_from_generic_array_annotations() {
    let source = r#"const box = new NativeBox(1); let mutable: Array<number>; const readonly = unknown as ReadonlyArray<number>; box.set(mutable); box.set(readonly);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_numbers".into(),
                1,
                false,
                vec![thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64))],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_numbers(box, mutable)"));
    assert!(rewritten.contains("__set_numbers(box, readonly)"));
}

#[test]
fn infers_external_overload_types_from_composed_expressions() {
    let source = r#"const box = new NativeBox(1); const n = 20 + 22; const s = "hel" + "lo"; const b = n > 0; const config = { n, nested: { text: s }, enabled: b }; box.set(n); box.set(s); box.set(b); box.set(Number("7")); box.set(`value-${s}`); box.set(true ? "yes" : "no"); box.set(config.n); box.set(config.nested.text); box.set(config.enabled); box.set(({ value: 7 }).value);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, n)"));
    assert!(rewritten.contains("__set_string(box, s)"));
    assert!(rewritten.contains("__set_boolean(box, b)"));
    assert!(rewritten.contains("__set_number(box, Number(\"7\"))"));
    assert!(rewritten.contains("__set_string(box, `value-${s}`)"));
    assert!(rewritten.contains("__set_string(box, true ? \"yes\" : \"no\")"));
    assert!(rewritten.contains("__set_number(box, config.n)"));
    assert!(rewritten.contains("__set_string(box, config.nested.text)"));
    assert!(rewritten.contains("__set_boolean(box, config.enabled)"));
    assert!(rewritten.contains("__set_number(box, ({ value: 7 }).value)"));
}

#[test]
fn infers_external_overloads_from_deterministic_operators() {
    let source = r#"const box = new NativeBox(1); const n = 4; const s = "value"; box.set("count=" + n); box.set(n + s); box.set(~n); box.set(n << 2); box.set(n | 1); box.set(typeof n); box.set(s || "fallback"); box.set(n ?? 0);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_string(box, \"count=\" + n)"));
    assert!(rewritten.contains("__set_string(box, n + s)"));
    assert!(rewritten.contains("__set_number(box, ~n)"));
    assert!(rewritten.contains("__set_number(box, n << 2)"));
    assert!(rewritten.contains("__set_number(box, n | 1)"));
    assert!(rewritten.contains("__set_string(box, typeof n)"));
    assert!(rewritten.contains("__set_string(box, s || \"fallback\")"));
    assert!(rewritten.contains("__set_number(box, n ?? 0)"));
}

#[test]
fn infers_external_overloads_from_fixed_result_standard_methods() {
    let source = r#"const box = new NativeBox(1); const text = " value "; const number = 42; const values = [1, 2]; box.set(text.trim()); box.set(number.toFixed(2)); box.set(values.join(",")); box.set(text.includes("a")); box.set(values.includes(2)); box.set(Math.max(1, 2)); box.set(parseInt("7")); box.set(JSON.stringify({ value: 1 })); box.set(Array.isArray(values));"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_string(box, text.trim())"));
    assert!(rewritten.contains("__set_string(box, number.toFixed(2))"));
    assert!(rewritten.contains("__set_string(box, values.join(\",\"))"));
    assert!(rewritten.contains("__set_boolean(box, text.includes(\"a\"))"));
    assert!(rewritten.contains("__set_boolean(box, values.includes(2))"));
    assert!(rewritten.contains("__set_number(box, Math.max(1, 2))"));
    assert!(rewritten.contains("__set_number(box, parseInt(\"7\"))"));
    assert!(rewritten.contains("__set_string(box, JSON.stringify({ value: 1 }))"));
    assert!(rewritten.contains("__set_boolean(box, Array.isArray(values))"));
}

#[test]
fn infers_external_overload_types_from_user_function_returns_and_forward_references() {
    let source = r#"const box = new NativeBox(1); const makeText = (): string => "text"; const makeFlag = function(): boolean { return true; }; const inferredFlag = () => true; box.set(makeNumber()); box.set(makeText()); box.set(makeFlag()); box.set(inferredNumber()); box.set(inferredText()); box.set(inferredFlag()); function makeNumber(): number { return 42; } function inferredNumber() { return 40 + 2; } function inferredText() { return forwardText(); } function forwardText() { return "text"; }"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, makeNumber())"));
    assert!(rewritten.contains("__set_string(box, makeText())"));
    assert!(rewritten.contains("__set_boolean(box, makeFlag())"));
    assert!(rewritten.contains("__set_number(box, inferredNumber())"));
    assert!(rewritten.contains("__set_string(box, inferredText())"));
    assert!(rewritten.contains("__set_boolean(box, inferredFlag())"));
}

#[test]
fn propagates_argument_types_through_passthrough_functions() {
    let source = r#"function forward(value) { return later(value); } function identity(value) { return value; } function later(value) { return identity(value); } const arrowForward = (value) => arrow(value); const arrow = (value) => value; const second = (first, value) => { return value; }; const expression = function(value) { return value; }; const numberBox = new NativeBox(forward(1)); const stringBox = new NativeBox(arrowForward("box")); numberBox.set(second(false, 2)); stringBox.set(expression("value"));"#;
    let constructors = vec![
        (1, "__ctor_string".into(), vec![thaw_hir::HirType::Str]),
        (1, "__ctor_number".into(), vec![thaw_hir::HirType::F64]),
    ];
    let rewritten = rewrite_external_class_methods(
        source,
        &[("addon".into(), "NativeBox".into(), constructors)],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("const numberBox = __ctor_number(forward(1))"));
    assert!(rewritten.contains("const stringBox = __ctor_string(arrowForward(\"box\"))"));
    assert!(rewritten.contains("__set_number(numberBox, second(false, 2))"));
    assert!(rewritten.contains("__set_string(stringBox, expression(\"value\"))"));
}

#[test]
fn propagates_common_argument_types_through_conditional_functions() {
    let source = r#"function forward(flag, first, second) { return choose(flag, first, second); } function choose(flag, first, second) { return flag ? first : second; } function branch(flag, first, second) { if (flag) { return first; } return second; } const arrowBranch = (flag, first, second) => { if (flag) { return first; } else { return second; } }; const logical = (first, second) => first ?? second; const numberBox = new NativeBox(forward(true, 1, 2)); const stringBox = new NativeBox(logical("a", "b")); numberBox.set(choose(false, 3, 4)); numberBox.set(branch(true, 5, 6)); stringBox.set(forward(false, "x", "y")); stringBox.set(arrowBranch(true, "m", "n"));"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![
                (1, "__ctor_string".into(), vec![thaw_hir::HirType::Str]),
                (1, "__ctor_number".into(), vec![thaw_hir::HirType::F64]),
            ],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("const numberBox = __ctor_number(forward(true, 1, 2))"));
    assert!(rewritten.contains("const stringBox = __ctor_string(logical(\"a\", \"b\"))"));
    assert!(rewritten.contains("__set_number(numberBox, choose(false, 3, 4))"));
    assert!(rewritten.contains("__set_number(numberBox, branch(true, 5, 6))"));
    assert!(rewritten.contains("__set_string(stringBox, forward(false, \"x\", \"y\"))"));
    assert!(rewritten.contains("__set_string(stringBox, arrowBranch(true, \"m\", \"n\"))"));
}

#[test]
fn selects_object_overloads_from_structural_property_types() {
    let source = r#"function makeNumeric(): { value: number } { return { value: 11 }; } const box = new NativeBox(1); const numeric = { value: 42 }; const textual = { value: "text" }; const choose = true; box.configure(numeric); box.configure(textual); box.configure({ value: 7 }); box.configure({ ["value"]: "computed" }); box.configure({ ...numeric }); box.configure({ ...numeric, value: "override" }); box.configure({ ...{ value: 9 } }); box.configure({ ...makeNumeric() }); box.configure({ ...(choose ? makeNumeric() : numeric) });"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::F64,
                )])],
            ),
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::Str,
                )])],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__configure_number(box, numeric)"));
    assert!(rewritten.contains("__configure_string(box, textual)"));
    assert!(rewritten.contains("__configure_number(box, { value: 7 })"));
    assert!(rewritten.contains("__configure_string(box, { [\"value\"]: \"computed\" })"));
    assert!(rewritten.contains("__configure_number(box, { ...numeric })"));
    assert!(rewritten.contains("__configure_string(box, { ...numeric, value: \"override\" })"));
    assert!(rewritten.contains("__configure_number(box, { ...{ value: 9 } })"));
    assert!(rewritten.contains("__configure_number(box, { ...makeNumeric() })"));
    assert!(
        rewritten.contains("__configure_number(box, { ...(choose ? makeNumeric() : numeric) })")
    );
}

#[test]
fn selects_object_overloads_from_explicit_object_types() {
    let source = r#"function makeText(): { value: string } { return unknown; } const box = new NativeBox(1); let numeric: { value: number }; const asserted = unknown as { value: string }; box.configure(numeric); box.configure(makeText()); box.configure(asserted);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::F64,
                )])],
            ),
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::Str,
                )])],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__configure_number(box, numeric)"));
    assert!(rewritten.contains("__configure_string(box, makeText())"));
    assert!(rewritten.contains("__configure_string(box, asserted)"));
}

#[test]
fn selects_object_overloads_through_named_types_and_forward_references() {
    let source = r#"const box = new NativeBox(1); let numeric: NumericConfig; function makeText(): TextConfig { return unknown; } box.configure(numeric); box.configure(makeText()); box.configure(unknown as NumericAlias); interface NumericConfig extends BaseConfig { nested: Detail; } type NumericAlias = NumericConfig; type TextConfig = { value: string }; interface BaseConfig { value: number; } interface BaseConfig { enabled: boolean; } type Detail = { label: string };"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::Str,
                )])],
            ),
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::F64,
                )])],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__configure_number(box, numeric)"));
    assert!(rewritten.contains("__configure_string(box, makeText())"));
    assert!(rewritten.contains("__configure_number(box, unknown as NumericAlias)"));
}

#[test]
fn selects_overloads_through_literal_unions_and_readonly_arrays() {
    let source = r#"type Mode = "read" | "write"; type Numbers = readonly number[]; function mode(): Mode { return unknown; } function numbers(): Numbers { return unknown; } const box = new NativeBox(1); box.set(mode()); box.set(numbers());"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_numbers".into(),
                1,
                false,
                vec![thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64))],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_string(box, mode())"));
    assert!(rewritten.contains("__set_numbers(box, numbers())"));
}

#[test]
fn selects_object_overloads_through_intersection_aliases() {
    let source = r#"type Base = { value: number }; type Numeric = Base & { enabled: boolean }; function config(): Numeric { return unknown; } const box = new NativeBox(1); box.configure(config());"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::Str,
                )])],
            ),
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::F64,
                )])],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__configure_number(box, config())"));
}

#[test]
fn selects_overloads_through_keyof_and_indexed_access_types() {
    let source = r#"type Config = { value: number; label: string }; function key(): keyof Config { return unknown; } function value(): Config["value"] { return unknown; } const box = new NativeBox(1); box.set(key()); box.set(value());"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_boolean".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_string(box, key())"));
    assert!(rewritten.contains("__set_number(box, value())"));
}

#[test]
fn selects_overloads_through_object_utility_types() {
    let source = r#"type Full = { value: number; label: string }; type Optional = { value?: number; label: string }; function picked(): Readonly<Pick<Full, "value">> { return unknown; } function all(): Pick<Full, keyof Full> { return unknown; } function omitted(): Omit<Full, "label"> { return unknown; } function required(): Required<Optional> { return unknown; } function recorded(): Record<"value", string> { return unknown; } const box = new NativeBox(1); box.configure(picked()); box.configure(all()); box.configure(omitted()); box.configure(required()); box.configure(recorded());"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::Str,
                )])],
            ),
            (
                "NativeBox".into(),
                "configure".into(),
                "__configure_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::Object(vec![(
                    "value".into(),
                    thaw_hir::HirType::F64,
                )])],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__configure_number(box, picked())"));
    assert!(rewritten.contains("__configure_number(box, all())"));
    assert!(rewritten.contains("__configure_number(box, omitted())"));
    assert!(rewritten.contains("__configure_number(box, required())"));
    assert!(rewritten.contains("__configure_string(box, recorded())"));
}

#[test]
fn tracks_assignment_flow_for_variables_and_nested_object_properties() {
    let source = r#"const box = new NativeBox(1); let value = 42; box.set(value); value = "text"; box.set(value); const config = { nested: { value: 1 }, direct: true }; config.nested.value = "nested"; config["direct"] = 7; box.set(config.nested.value); box.set(config.direct);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, value); value = \"text\""));
    assert!(rewritten.contains("__set_string(box, value); const config"));
    assert!(rewritten.contains("__set_string(box, config.nested.value)"));
    assert!(rewritten.contains("__set_number(box, config.direct)"));
}

#[test]
fn joins_if_branch_types_and_discards_conflicting_facts() {
    let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; if (flag) { stable = 2; } else { stable = 3; } box.set(stable); let conflict = 1; if (flag) { conflict = "text"; box.set(conflict); } else { conflict = 2; box.set(conflict); } box.set(conflict); let oneSided = 1; if (flag) { oneSided = "changed"; } box.set(oneSided);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("conflict = \"text\"; __set_string(box, conflict)"));
    assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, oneSided)"));
}

#[test]
fn joins_switch_fallthrough_break_and_no_match_paths() {
    let source = r#"const choice = 1; const flag = true; const box = new NativeBox(1); let stable = 1; switch (choice) { case 0: stable = 2; break; default: stable = 3; } box.set(stable); let conflict = 1; switch (choice) { case 0: conflict = "text"; break; default: conflict = 2; } box.set(conflict); let noDefault = 1; switch (choice) { case 0: noDefault = "text"; break; } box.set(noDefault); let fallen = 1; switch (choice) { case 0: fallen = "temporary"; case 1: fallen = 2; break; default: fallen = 3; } box.set(fallen); let guarded = 1; switch (choice) { case 0: if (flag) { guarded = "text"; break; guarded = 4; } guarded = 2; break; default: guarded = 3; } box.set(guarded); let loopBreak = 1; switch (choice) { case 0: while (flag) { break; } loopBreak = 2; break; default: loopBreak = 3; } box.set(loopBreak); let stableCallback = (): void => {}; switch (choice) { case 0: stableCallback = (): void => {}; break; default: stableCallback = (): void => {}; } box.use(stableCallback); let conflictCallback = (): void => {}; switch (choice) { case 0: conflictCallback = 1; break; default: conflictCallback = (): void => {}; } box.use(conflictCallback); let stableBox = new NativeBox(1); switch (choice) { case 0: stableBox = new NativeBox(2); break; default: stableBox = new NativeBox(3); } stableBox.get(); let conflictBox = new NativeBox(1); switch (choice) { case 0: conflictBox = "text"; break; default: conflictBox = new NativeBox(3); } conflictBox.get();"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "use".into(),
                "__use_value".into(),
                1,
                false,
                vec![thaw_hir::HirType::Json],
            ),
            (
                "NativeBox".into(),
                "use".into(),
                "__use_callback".into(),
                1,
                true,
                vec![thaw_hir::HirType::Json],
            ),
            (
                "NativeBox".into(),
                "get".into(),
                "__get".into(),
                0,
                false,
                Vec::new(),
            ),
        ],
    )
    .unwrap();

    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("__set_unknown(box, conflict)"));
    assert!(rewritten.contains("__set_unknown(box, noDefault)"));
    assert!(rewritten.contains("__set_number(box, fallen)"));
    assert!(rewritten.contains("__set_unknown(box, guarded)"));
    assert!(rewritten.contains("__set_number(box, loopBreak)"));
    assert!(rewritten.contains("__use_callback(box, stableCallback)"));
    assert!(rewritten.contains("__use_value(box, conflictCallback)"));
    assert!(rewritten.contains("__get(stableBox)"));
    assert!(rewritten.contains("conflictBox.get()"));
}

#[test]
fn joins_while_and_for_types_against_the_zero_iteration_path() {
    let source = r#"const box = new NativeBox(1); const flag = true; let stable = 1; while (flag) { stable = 2; box.set(stable); break; } box.set(stable); let changed = 1; while (flag) { changed = "text"; box.set(changed); break; } box.set(changed); let loopValue = 1; for (let index = 0; index < 1; index = index + 1) { box.set(index); loopValue = "loop"; box.set(loopValue); } box.set(loopValue);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("stable = 2; __set_number(box, stable)"));
    assert!(rewritten.contains("} __set_number(box, stable)"));
    assert!(rewritten.contains("changed = \"text\"; __set_string(box, changed)"));
    assert!(rewritten.contains("} __set_unknown(box, changed)"));
    assert!(rewritten.contains("__set_number(box, index)"));
    assert!(rewritten.contains("loopValue = \"loop\"; __set_string(box, loopValue)"));
    assert!(rewritten.contains("} __set_unknown(box, loopValue)"));
}

#[test]
fn joins_try_catch_paths_and_applies_finally_to_every_exit() {
    let source = r#"const box = new NativeBox(1); let stable = 1; try { stable = 2; } catch (error) { stable = 3; } box.set(stable); let conflict = 1; try { conflict = "try"; box.set(conflict); } catch (error) { conflict = 2; box.set(conflict); } box.set(conflict); let catchInput = 1; try { catchInput = "changed"; throw "fail"; } catch (error) { box.set(catchInput); } let finalized = 1; try { finalized = "try"; } catch (error) { finalized = true; } finally { finalized = 7; box.set(finalized); } box.set(finalized);"#;
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[
            (
                "NativeBox".into(),
                "set".into(),
                "__set_unknown".into(),
                1,
                false,
                vec![thaw_hir::HirType::Bool],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "NativeBox".into(),
                "set".into(),
                "__set_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
    )
    .unwrap();
    assert!(rewritten.contains("__set_number(box, stable)"));
    assert!(rewritten.contains("conflict = \"try\"; __set_string(box, conflict)"));
    assert!(rewritten.contains("conflict = 2; __set_number(box, conflict)"));
    assert!(rewritten.contains("} __set_unknown(box, conflict)"));
    assert!(rewritten.contains("catch (error) { __set_unknown(box, catchInput)"));
    assert!(rewritten.contains("finalized = 7; __set_number(box, finalized)"));
    assert!(rewritten.ends_with("__set_number(box, finalized);"));
}

/// Two same-arity overloads whose shared parameter position declares
/// genuinely different *native scalar* types (real example: `ms`'s
/// `(value: number, options?): string` vs `(value: StringValue):
/// number`) are a real conflict when no argument's type can be
/// determined at all -- picking either one forces a possibly-lossy
/// coercion decision with no basis for it, so the call is left
/// unrewritten instead of guessing.
#[test]
fn defers_a_same_arity_tie_between_conflicting_scalar_overloads() {
    let source = "describe(untyped);";
    let rewritten = rewrite_fallback_function_overloads(
        source,
        &[
            (
                "describe".into(),
                "__describe_number".into(),
                1,
                1,
                vec![thaw_hir::HirType::F64],
                None,
            ),
            (
                "describe".into(),
                "__describe_string".into(),
                1,
                1,
                vec![thaw_hir::HirType::Str],
                None,
            ),
        ],
    )
    .unwrap();
    assert_eq!(rewritten, "describe(untyped);");
}

/// Two same-arity overloads that only differ through an opaque
/// `Json`/`JsValue` parameter -- one that's never actually coerced
/// into anything more specific either way -- are safely
/// interchangeable even with no argument type information at all (real
/// example: zod's `string(params?): ZodString` alongside its own
/// generic `string<T extends string>(params?): $ZodType<T,T>`, and
/// pino's `pino(...)`, whose own `.d.ts` bundles the exact same
/// overload twice): picking the first one, matching the ordinary tie
/// rule, calls the same real underlying JS function with the same
/// untouched value regardless of which one is chosen.
#[test]
fn rewrites_a_same_arity_tie_between_opaque_overloads() {
    let source = "describe(); describe(untyped);";
    let rewritten = rewrite_fallback_function_overloads(
        source,
        &[
            (
                "describe".into(),
                "__describe_first".into(),
                0,
                1,
                vec![thaw_hir::HirType::Json],
                None,
            ),
            (
                "describe".into(),
                "__describe_second".into(),
                0,
                1,
                vec![thaw_hir::HirType::Json],
                None,
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "__describe_first(); __describe_first(untyped);"
    );
}

/// A tied pair where only *one* side is opaque still prefers that
/// side: calling it never actually depends on the argument's real
/// type, so it's a universally safe representative for the whole tied
/// set regardless of what its scalar-typed sibling declares (unlike
/// two *different* scalar types tying with no opaque escape at all,
/// which do defer -- see `defers_a_same_arity_tie_between_conflicting_
/// scalar_overloads`).
#[test]
fn rewrites_a_same_arity_tie_between_a_scalar_and_an_opaque_overload() {
    let source = "describe(untyped);";
    let rewritten = rewrite_fallback_function_overloads(
        source,
        &[
            (
                "describe".into(),
                "__describe_number".into(),
                1,
                1,
                vec![thaw_hir::HirType::F64],
                None,
            ),
            (
                "describe".into(),
                "__describe_opaque".into(),
                1,
                1,
                vec![thaw_hir::HirType::Json],
                None,
            ),
        ],
    )
    .unwrap();
    assert_eq!(rewritten, "__describe_opaque(untyped);");
}

/// The same unresolvable argument does *not* block a rewrite when only
/// one overload matches by arity at all -- there is nothing else it
/// could be, so this isn't a guess the way choosing among several
/// same-arity candidates would be (matches `selects_fallback_function_
/// overloads_by_arity_range`'s arity-only disambiguation, just with an
/// unknown argument type instead of a known one).
#[test]
fn rewrites_a_sole_arity_match_even_when_the_argument_type_is_unknown() {
    let source = "makeId(untyped);";
    let rewritten = rewrite_fallback_function_overloads(
        source,
        &[
            (
                "makeId".into(),
                "__makeId_default".into(),
                1,
                1,
                vec![thaw_hir::HirType::F64],
                None,
            ),
            (
                "makeId".into(),
                "__makeId_buffer".into(),
                2,
                2,
                vec![thaw_hir::HirType::F64, thaw_hir::HirType::F64],
                None,
            ),
        ],
    )
    .unwrap();
    assert_eq!(rewritten, "__makeId_default(untyped);");
}
