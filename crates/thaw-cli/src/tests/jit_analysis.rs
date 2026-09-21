#[test]
fn recognizes_primitive_and_mutual_recursion_for_jit() {
    let source = "function isEven(value) { return value === 0 ? true : !isEven(value - 1); } function punctuate(count, value) { return count <= 0 ? value : punctuate(count - 1, value + '!'); } function ping(value) { return value <= 0 ? 0 : pong(value - 1) + 1; } function pong(value) { return value <= 0 ? 0 : ping(value - 1) + 1; } module.exports = { isEven, punctuate, ping };";
    let function = |name: &str, params, ret| thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: name.into(),
        generic: None,
        params,
        required_params: if name == "punctuate" { 2 } else { 1 },
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(ret),
    };
    let boolean_function = function(
        "isEven",
        vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        )],
        thaw_hir::HirType::Bool,
    );
    assert!(jit_numeric_export(source, "isEven", false, &boolean_function,).is_some());
    assert!(jit_numeric_export(
        source,
        "punctuate",
        false,
        &function(
            "punctuate",
            vec![
                (
                    "count".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                ),
                (
                    "value".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
                ),
            ],
            thaw_hir::HirType::Str,
        ),
    )
    .is_some());
    assert!(jit_numeric_export(
        source,
        "ping",
        false,
        &function(
            "ping",
            vec![(
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            )],
            thaw_hir::HirType::F64,
        ),
    )
    .is_some());
}

#[test]
fn recognizes_tagged_statement_returns_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "choose".into(),
        generic: None,
        params: vec![
            (
                "useLabel".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
            ),
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
            thaw_hir::HirType::F64,
            thaw_hir::HirType::Str,
        ])),
    };
    assert!(jit_numeric_export(
        "function choose(useLabel, value) { if (useLabel) return 'value=' + value; return value + 1; } module.exports = { choose };",
        "choose",
        false,
        &function,
    )
    .is_some());
    let switch = thaw_bridge::DtsFunction {
        name: "chooseSwitch".into(),
        params: vec![
            (
                "mode".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            function.params[1].clone(),
        ],
        ..function
    };
    assert!(jit_numeric_export(
        "function chooseSwitch(mode, value) { switch (mode) { case 0: return value + 1; case 1: return 'value=' + value; default: return 'other'; } } module.exports = { chooseSwitch };",
        "chooseSwitch",
        false,
        &switch,
    )
    .is_some());
    let narrow = thaw_bridge::DtsFunction {
        name: "narrow".into(),
        ..switch.clone()
    };
    assert!(jit_numeric_export(
        "function narrow(mode, value) { let result = value + 1; if (mode) result = 'next'; return typeof result === 'number' ? result + 1 : result + '!'; } module.exports = { narrow };",
        "narrow",
        false,
        &narrow,
    )
    .is_some());
    let narrow_loop = thaw_bridge::DtsFunction {
        name: "narrowLoop".into(),
        ..narrow
    };
    assert!(jit_numeric_export(
        "function narrowLoop(mode, value) { let result = value + 1; let index = 0; while (index < 1) { if (typeof result === 'number') result = result + 1; else result = result + '!'; index++; } return result; } module.exports = { narrowLoop };",
        "narrowLoop",
        false,
        &narrow_loop,
    )
    .is_some());
}

#[test]
fn jit_copies_a_narrowed_mixed_array_union() {
    let union = thaw_hir::HirType::Union(vec![
        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)),
        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Str)),
        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Bool)),
        thaw_hir::HirType::Str,
    ]);
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "slice".into(),
        generic: None,
        params: vec![("value".into(), thaw_bridge::DtsType::Native(union.clone()))],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(union),
    };
    assert!(jit_numeric_export(
        "function slice(value) { if (Array.isArray(value)) return value.slice(1); return value.slice(1); } module.exports = { slice };",
        "slice",
        false,
        &function,
    )
    .is_some());

    let function = thaw_bridge::DtsFunction {
        name: "concat".into(),
        ..function
    };
    assert!(jit_numeric_export(
        "function concat(value) { if (Array.isArray(value)) return value.concat(value); return value.concat(value); } module.exports = { concat };",
        "concat",
        false,
        &function,
    )
    .is_some());

    let function = thaw_bridge::DtsFunction {
        name: "at".into(),
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
            thaw_hir::HirType::F64,
            thaw_hir::HirType::Str,
            thaw_hir::HirType::Bool,
        ])),
        ..function
    };
    assert!(jit_numeric_export(
        "function at(value) { if (Array.isArray(value)) return value.at(-1); return value; } module.exports = { at };",
        "at",
        false,
        &function,
    )
    .is_some());

    for method in ["toReversed", "reverse", "toSorted", "sort"] {
        let reversed = thaw_bridge::DtsFunction {
            name: method.into(),
            ret: function.params[0].1.clone(),
            ..function.clone()
        };
        let source = format!(
            "function {method}(value) {{ if (Array.isArray(value)) return value.{method}(); return value; }} module.exports = {{ {method} }};"
        );
        assert!(jit_numeric_export(&source, method, false, &reversed).is_some());
    }

    let needle = thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
        thaw_hir::HirType::F64,
        thaw_hir::HirType::Str,
        thaw_hir::HirType::Bool,
    ]));
    let fill = thaw_bridge::DtsFunction {
        name: "fill".into(),
        params: vec![
            function.params[0].clone(),
            ("replacement".into(), needle.clone()),
        ],
        required_params: 2,
        ret: function.params[0].1.clone(),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function fill(value, replacement) { if (Array.isArray(value)) return value.fill(replacement, 1); return value; } module.exports = { fill };",
        "fill",
        false,
        &fill,
    )
    .is_some());
    let copy_within = thaw_bridge::DtsFunction {
        name: "copyWithin".into(),
        ret: function.params[0].1.clone(),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function copyWithin(value) { if (Array.isArray(value)) return value.copyWithin(0, 1); return value; } module.exports = { copyWithin };",
        "copyWithin",
        false,
        &copy_within,
    )
    .is_some());
    let with = thaw_bridge::DtsFunction {
        name: "withValue".into(),
        params: vec![
            function.params[0].clone(),
            ("replacement".into(), needle.clone()),
        ],
        required_params: 2,
        ret: function.params[0].1.clone(),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function withValue(value, replacement) { if (Array.isArray(value)) return value.with(1, replacement); return value; } module.exports = { withValue };",
        "withValue",
        false,
        &with,
    )
    .is_some());
    let insert = thaw_bridge::DtsFunction {
        name: "insert".into(),
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..with.clone()
    };
    assert!(jit_numeric_export(
        "function insert(value, replacement) { if (Array.isArray(value)) return value.push(replacement); return 0; } module.exports = { insert };",
        "insert",
        false,
        &insert,
    )
    .is_some());
    let prepend = thaw_bridge::DtsFunction {
        name: "prepend".into(),
        ..insert
    };
    assert!(jit_numeric_export(
        "function prepend(value, replacement) { if (Array.isArray(value)) return value.unshift(replacement); return 0; } module.exports = { prepend };",
        "prepend",
        false,
        &prepend,
    )
    .is_some());
    let remove = thaw_bridge::DtsFunction {
        name: "popValue".into(),
        params: vec![function.params[0].clone()],
        required_params: 1,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
            thaw_hir::HirType::F64,
            thaw_hir::HirType::Str,
            thaw_hir::HirType::Bool,
        ])),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function popValue(value) { if (Array.isArray(value)) return value.pop(); return value; } module.exports = { popValue };",
        "popValue",
        false,
        &remove,
    )
    .is_some());
    let shift_value = thaw_bridge::DtsFunction {
        name: "shiftValue".into(),
        ..remove
    };
    assert!(jit_numeric_export(
        "function shiftValue(value) { if (Array.isArray(value)) return value.shift(); return value; } module.exports = { shiftValue };",
        "shiftValue",
        false,
        &shift_value,
    )
    .is_some());
    let set_value = thaw_bridge::DtsFunction {
        name: "setValue".into(),
        params: vec![
            function.params[0].clone(),
            (
                "index".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            ("replacement".into(), needle.clone()),
        ],
        required_params: 3,
        ret: needle.clone(),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function setValue(value, index, replacement) { if (Array.isArray(value)) return value[index] = replacement; return replacement; } module.exports = { setValue };",
        "setValue",
        false,
        &set_value,
    )
    .is_some());
    for method in ["splice", "toSpliced"] {
        let splice = thaw_bridge::DtsFunction {
            name: method.into(),
            params: vec![
                function.params[0].clone(),
                ("replacement".into(), needle.clone()),
            ],
            required_params: 2,
            ret: function.params[0].1.clone(),
            ..function.clone()
        };
        let source = format!(
            "function {method}(value, replacement) {{ if (Array.isArray(value)) return value.{method}(1, 1, replacement); return value; }} module.exports = {{ {method} }};"
        );
        assert!(jit_numeric_export(&source, method, false, &splice).is_some());
    }
    for method in ["some", "every"] {
        let predicate = thaw_bridge::DtsFunction {
            name: method.into(),
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
            ..function.clone()
        };
        let source = format!(
            "function {method}(value) {{ if (Array.isArray(value)) return value.{method}(item => item); return false; }} module.exports = {{ {method} }};"
        );
        assert!(jit_numeric_export(&source, method, false, &predicate).is_some());
    }
    for method in ["findIndex", "findLastIndex"] {
        let index = thaw_bridge::DtsFunction {
            name: method.into(),
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ..function.clone()
        };
        let source = format!(
            "function {method}(value) {{ if (Array.isArray(value)) return value.{method}(item => item); return -1; }} module.exports = {{ {method} }};"
        );
        assert!(jit_numeric_export(&source, method, false, &index).is_some());
    }
    for method in ["find", "findLast"] {
        let found = thaw_bridge::DtsFunction {
            name: method.into(),
            ..function.clone()
        };
        let source = format!(
            "function {method}(value) {{ if (Array.isArray(value)) return value.{method}(item => item); return value; }} module.exports = {{ {method} }};"
        );
        assert!(jit_numeric_export(&source, method, false, &found).is_some());
    }
    let filter = thaw_bridge::DtsFunction {
        name: "filter".into(),
        ret: function.params[0].1.clone(),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function filter(value) { if (Array.isArray(value)) return value.filter(item => item); return value; } module.exports = { filter };",
        "filter",
        false,
        &filter,
    )
    .is_some());
    for method in ["some", "every"] {
        let predicate = thaw_bridge::DtsFunction {
            name: method.into(),
            params: vec![
                function.params[0].clone(),
                ("needle".into(), needle.clone()),
            ],
            required_params: 2,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
            ..function.clone()
        };
        for comparison in ["==", "===", "!=", "!==", "<", "<=", ">", ">="] {
            let source = format!(
                "function {method}(value, needle) {{ if (Array.isArray(value)) return value.{method}(item => item {comparison} needle); return false; }} module.exports = {{ {method} }};"
            );
            assert!(jit_numeric_export(&source, method, false, &predicate).is_some());
        }
    }
    for method in ["find", "findIndex", "findLast", "findLastIndex", "filter"] {
        let compared = thaw_bridge::DtsFunction {
            name: method.into(),
            params: vec![
                function.params[0].clone(),
                ("needle".into(), needle.clone()),
            ],
            required_params: 2,
            ret: match method {
                "findIndex" | "findLastIndex" => {
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64)
                }
                "filter" => function.params[0].1.clone(),
                _ => function.ret.clone(),
            },
            ..function.clone()
        };
        let fallback = if method == "findIndex" || method == "findLastIndex" {
            "-1"
        } else {
            "value"
        };
        let source = format!(
            "function {method}(value, needle) {{ if (Array.isArray(value)) return value.{method}(item => item === needle); return {fallback}; }} module.exports = {{ {method} }};"
        );
        assert!(jit_numeric_export(&source, method, false, &compared).is_some());
    }
    for (callback, element) in [
        ("Number", thaw_hir::HirType::F64),
        ("Boolean", thaw_hir::HirType::Bool),
        ("String", thaw_hir::HirType::Str),
    ] {
        let map = thaw_bridge::DtsFunction {
            name: format!("map{callback}"),
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(element))),
            ..function.clone()
        };
        let source = format!(
            "function map{callback}(value) {{ if (Array.isArray(value)) return value.map({callback}); return []; }} module.exports = {{ map{callback} }};"
        );
        assert!(jit_numeric_export(&source, &map.name, false, &map).is_some());
    }
    let identity = thaw_bridge::DtsFunction {
        name: "mapIdentity".into(),
        ret: function.params[0].1.clone(),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function mapIdentity(value) { if (Array.isArray(value)) return value.map(item => item); return value; } module.exports = { mapIdentity };",
        "mapIdentity",
        false,
        &identity,
    )
    .is_some());
    for (name, callback, element) in [
        ("mapNumber", "item => Number(item)", thaw_hir::HirType::F64),
        ("mapBoolean", "item => !!item", thaw_hir::HirType::Bool),
        (
            "mapString",
            "(item, index) => String(item) + ':' + String(index)",
            thaw_hir::HirType::Str,
        ),
        (
            "mapWithArray",
            "(item, index, values) => Number(item) + index + values.length",
            thaw_hir::HirType::F64,
        ),
    ] {
        let map = thaw_bridge::DtsFunction {
            name: name.into(),
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(element))),
            ..function.clone()
        };
        let source = format!(
            "function {name}(value) {{ if (Array.isArray(value)) return value.map({callback}); return []; }} module.exports = {{ {name} }};"
        );
        assert!(
            jit_numeric_export(&source, name, false, &map).is_some(),
            "{name}"
        );
    }
    let captured_map = thaw_bridge::DtsFunction {
        name: "mapCaptured".into(),
        params: vec![
            function.params[0].clone(),
            (
                "offset".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..function.clone()
    };
    assert!(jit_numeric_export(
        "function mapCaptured(value, offset) { if (Array.isArray(value)) return value.map(item => Number(item) + offset); return []; } module.exports = { mapCaptured };",
        "mapCaptured",
        false,
        &captured_map,
    )
    .is_some());
    for (method, ret) in [
        (
            "some",
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ),
        (
            "every",
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ),
        ("find", function.ret.clone()),
        (
            "findIndex",
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ),
        ("findLast", function.ret.clone()),
        (
            "findLastIndex",
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ),
        ("filter", function.params[0].1.clone()),
    ] {
        let scan = thaw_bridge::DtsFunction {
            name: method.into(),
            ret,
            ..function.clone()
        };
        let fallback = match method {
            "some" | "every" => "false",
            "findIndex" | "findLastIndex" => "-1",
            _ => "value",
        };
        let source = format!(
            "function {method}(value) {{ if (Array.isArray(value)) return value.{method}((item, index, values) => Number(item) + index >= values.length); return {fallback}; }} module.exports = {{ {method} }};"
        );
        assert!(
            jit_numeric_export(&source, method, false, &scan).is_some(),
            "{method}"
        );
    }
    for method in ["reduce", "reduceRight"] {
        let reduce = thaw_bridge::DtsFunction {
            name: method.into(),
            params: vec![
                function.params[0].clone(),
                (
                    "initial".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                ),
                (
                    "factor".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                ),
            ],
            required_params: 3,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ..function.clone()
        };
        let source = format!(
            "function {method}(value, initial, factor) {{ if (Array.isArray(value)) return value.{method}((accumulator, item, index, values) => accumulator * factor + Number(item) + index + values.length, initial); return initial; }} module.exports = {{ {method} }};"
        );
        assert!(
            jit_numeric_export(&source, method, false, &reduce).is_some(),
            "{method}"
        );
    }
    for method in ["reduceFirst", "reduceRightFirst"] {
        let reduce = thaw_bridge::DtsFunction {
            name: method.into(),
            params: vec![
                function.params[0].clone(),
                (
                    "enabled".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
                ),
            ],
            required_params: 2,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
                thaw_hir::HirType::F64,
                thaw_hir::HirType::Str,
                thaw_hir::HirType::Bool,
            ])),
            ..function.clone()
        };
        let operation = if method == "reduceFirst" {
            "reduce"
        } else {
            "reduceRight"
        };
        let source = format!(
            "function {method}(value, enabled) {{ if (Array.isArray(value)) return value.{operation}((accumulator, item) => enabled ? accumulator + item : String(accumulator) + String(item)); return value; }} module.exports = {{ {method} }};"
        );
        assert!(
            jit_numeric_export(&source, method, false, &reduce).is_some(),
            "{method}"
        );
    }
    let search = thaw_bridge::DtsFunction {
        name: "includes".into(),
        params: vec![function.params[0].clone(), ("needle".into(), needle)],
        required_params: 2,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ..function
    };
    assert!(jit_numeric_export(
        "function includes(value, needle) { if (Array.isArray(value)) return value.includes(needle); return false; } module.exports = { includes };",
        "includes",
        false,
        &search,
    )
    .is_some());
    for method in ["indexOf", "lastIndexOf"] {
        let function = thaw_bridge::DtsFunction {
            name: method.into(),
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ..search.clone()
        };
        let source = format!(
            "function {method}(value, needle) {{ if (Array.isArray(value)) return value.{method}(needle); return -1; }} module.exports = {{ {method} }};"
        );
        assert!(jit_numeric_export(&source, method, false, &function).is_some());
    }
}

#[test]
fn jit_joins_aggregate_only_union_branches() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "chooseArray".into(),
        generic: None,
        params: vec![(
            "flag".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        )],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
            thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)),
            thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Str)),
        ])),
    };
    assert!(jit_numeric_export(
        "function chooseArray(flag) { return flag ? [1, 2] : ['a', 'b']; } module.exports = { chooseArray };",
        "chooseArray",
        false,
        &function,
    )
    .is_some());
}

#[test]
fn jit_tags_fixed_aggregate_union_results() {
    for (name, body, aggregate) in [
        (
            "fixedArray",
            "function fixedArray() { return [1, 2]; } module.exports = { fixedArray };",
            thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)),
        ),
        (
            "fixedRecord",
            "function fixedRecord() { return { count: 3 }; } module.exports = { fixedRecord };",
            thaw_hir::HirType::Dictionary(Box::new(thaw_hir::HirType::F64)),
        ),
    ] {
        let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
            name: name.into(),
            generic: None,
            params: Vec::new(),
            required_params: 0,
            rest_param: None,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(vec![
                aggregate,
                thaw_hir::HirType::Str,
            ])),
        };
        assert!(jit_numeric_export(body, name, false, &function).is_some());
    }
}

#[test]
fn recognizes_set_constructor_and_methods_for_jit() {
    let function = |name: &str| thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: name.into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let export = jit_numeric_export(
        "function roundTrip() { const s = new Set([1, 2, 3]); s.add(4); s.delete(1); return s.has(2) ? (s.has(3) ? 1 : 2) : 3; } module.exports = { roundTrip };",
        "roundTrip",
        false,
        &function("roundTrip"),
    );
    let export = export.expect("Set operations should be JIT-specializable");
    assert!(export.contains("dsempty"), "expected a dictionary Set: {export}");
    assert!(export.contains("dsset"), "expected Set.add -> dsset: {export}");
    assert!(export.contains("dhasown"), "expected Set.has -> dhasown: {export}");
    assert!(export.contains("ddelete"), "expected Set.delete -> ddelete: {export}");
    assert!(export.contains("dsl"), "expected a materialized Set local: {export}");
}

#[test]
fn recognizes_set_composition_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "combine".into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    for (operation, token) in [
        ("union", "setunion"),
        ("intersection", "setintersection"),
        ("difference", "setdifference"),
        ("symmetricDifference", "setsymmetricdiff"),
    ] {
        let source = format!(
            "function combine() {{ const x = new Set([1, 2]); const y = new Set([2, 3]); const z = x.{operation}(y); return z.has(2) ? 1 : 0; }} module.exports = {{ combine }};"
        );
        let export = jit_numeric_export(&source, "combine", false, &function)
            .unwrap_or_else(|| panic!("{operation} should be JIT-specializable"));
        assert!(export.contains(token), "expected {token}: {export}");
    }
    for (operation, token) in [
        ("isSubsetOf", "setissubset"),
        ("isSupersetOf", "setissuperset"),
        ("isDisjointFrom", "setisdisjoint"),
    ] {
        let source = format!(
            "function combine() {{ const x = new Set([1, 2]); const y = new Set([2, 3]); return x.{operation}(y) ? 1 : 0; }} module.exports = {{ combine }};"
        );
        let export = jit_numeric_export(&source, "combine", false, &function)
            .unwrap_or_else(|| panic!("{operation} should be JIT-specializable"));
        assert!(export.contains(token), "expected {token}: {export}");
    }
    let export = jit_numeric_export(
        "function combine() { const x = new Set([1, 2]); const y = new Map(); y.set(2, 'two'); y.set(3, 'three'); const z = x.union(y); return z.has(3) ? 1 : 0; } module.exports = { combine };",
        "combine",
        true,
        &function,
    )
    .expect("Map should be accepted as a native Set-like value");
    assert!(export.contains("setunion"), "expected Set-like union: {export}");
}

#[test]
fn recognizes_opaque_set_like_arguments_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "combine".into(),
        generic: None,
        params: vec![(
            "other".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::JsValue),
        )],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let export = jit_numeric_export(
        "module.exports.combine = function(other) { const x = new Set(['a', 'b']); const union = x.union(other); return union.has('c') ? 1 : 0; };",
        "combine",
        false,
        &function,
    )
    .unwrap_or_else(|| panic!("{}", jit_rejection_reason("module.exports.combine = function(other) { const x = new Set(['a', 'b']); const union = x.union(other); return union.has('c') ? 1 : 0; };", &function)));
    assert!(export.contains("h0,setlike,setunion"), "{export}");
}

#[test]
fn recognizes_set_size_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "count".into(),
        generic: None,
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        )],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let export = jit_numeric_export(
        "function count(value) { const s = new Set([1, 2]); s.add(value); return s.size; } module.exports = { count };",
        "count",
        false,
        &function,
    );
    let export = export.expect("Set.size should be JIT-specializable");
    assert!(export.contains("dlen"), "expected Set.size -> dlen: {export}");
}


#[test]
fn recognizes_set_for_of_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "findValue".into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let export = jit_numeric_export(
        "function findValue() { const s = new Set(['a', 'b', 'c']); for (const value of s) { if (value === 'b') return 1; } return 0; } module.exports = { findValue };",
        "findValue",
        false,
        &function,
    );
    let export = export.expect("for...of over a Set should be JIT-specializable");
    assert!(export.contains("dkeys"), "expected Set iteration -> dkeys: {export}");
}

#[test]
fn recognizes_set_from_iterable_for_jit() {
    let function = |name: &str| thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: name.into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let from_array = jit_numeric_export(
        "function fromArray() { const values = ['a', 'b', 'c']; const s = new Set(values); return s.has('b') ? 1 : 0; } module.exports = { fromArray };",
        "fromArray",
        false,
        &function("fromArray"),
    );
    let from_array = from_array.expect("new Set(array) should be JIT-specializable");
    assert!(
        from_array.contains("setfromarray"),
        "expected new Set(array) -> setfromarray: {from_array}"
    );
    let from_string = jit_numeric_export(
        "function fromString() { const text = 'abc'; const s = new Set(text); return s.has('c') ? 1 : 0; } module.exports = { fromString };",
        "fromString",
        false,
        &function("fromString"),
    );
    let from_string = from_string.expect("new Set(string) should be JIT-specializable");
    assert!(
        from_string.contains("setfromarray"),
        "expected new Set(string) -> setfromarray: {from_string}"
    );
}

#[test]
fn recognizes_map_get_set_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "cache".into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let export = jit_numeric_export(
        "function cache() { const m = new Map(); m.set('a', 1); m.set('b', 2); return m.get('a') + (m.size === 2 ? 10 : 0); } module.exports = { cache };",
        "cache",
        false,
        &function,
    );
    let export = export.expect("Map get/set/size should be JIT-specializable");
    assert!(export.contains("dnempty"), "expected empty Map -> dnempty: {export}");
    assert!(export.contains("dnget"), "expected Map.get -> dnget: {export}");
    assert!(export.contains("dnset"), "expected Map.set -> dnset: {export}");
    assert!(export.contains("dlen"), "expected Map.size -> dlen: {export}");
}


#[test]
fn recognizes_map_keys_values_entries_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "spread".into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
    };
    let export = jit_numeric_export(
        "function spread() { const m = new Map(); m.set('a', 1); m.set('b', 2); const k = m.keys(); return k.join(','); } module.exports = { spread };",
        "spread",
        false,
        &function,
    );
    let export = export.expect("Map.keys should be JIT-specializable");
    assert!(export.contains("dkeys"), "expected Map.keys -> dkeys: {export}");
    let values = jit_numeric_export(
        "function spread() { const m = new Map(); m.set('a', 1); const v = m.values(); return v.join(','); } module.exports = { spread };",
        "spread",
        false,
        &thaw_bridge::DtsFunction {
            param_field_constraints: Vec::new(),
            name: "spread".into(),
            generic: None,
            params: vec![],
            required_params: 0,
            rest_param: None,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        },
    );
    let values = values.expect("Map.values should be JIT-specializable");
    assert!(values.contains("dnvalues"), "expected Map.values -> dnvalues: {values}");
}

#[test]
fn recognizes_direct_map_entry_iteration_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "entries".into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let export = jit_numeric_export(
        "function entries() { const m = new Map(); m.set('a', 1); m.set('b', 2); let total = 0; for (const [key, value] of m) total += value; return total; } module.exports = { entries };",
        "entries",
        false,
        &function,
    )
    .expect("direct Map entry iteration should be JIT-specializable");
    assert!(export.contains("dkeys"), "expected Map keys snapshot: {export}");
    assert!(export.contains("dnget"), "expected Map value lookup: {export}");
}

#[test]
fn recognizes_map_and_set_for_each_for_jit() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "total".into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let map = jit_numeric_export(
        "function total() { const m = new Map(); m.set('a', 1); m.set('b', 2); let result = 0; m.forEach((value, key, map) => { const doubled = value * 2; map.has(key); result += doubled; }); return result; } module.exports = { total };",
        "total",
        false,
        &function,
    )
    .expect("Map.forEach should be JIT-specializable");
    assert!(map.contains("dnget"), "expected Map value lookup: {map}");
    let set = jit_numeric_export(
        "function total() { const s = new Set(['a', 'b']); let result = 0; s.forEach((value, key, set) => { let length = 0; let extra = 1; length = value.length; set.has(key); result += length + extra; return value; }); return result; } module.exports = { total };",
        "total",
        false,
        &function,
    )
    .expect("Set.forEach should be JIT-specializable");
    assert!(set.contains("dkeys"), "expected Set keys snapshot: {set}");
}

#[test]
fn reorders_pure_commutative_arithmetic_to_fit_jit_registers() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "deep".into(),
        generic: None,
        params: vec![],
        required_params: 0,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    assert!(jit_numeric_export(
        "function deep() { return 1 + (2 + (3 + (4 + (5 + (6 + (7 + (8 + (9 + 10)))))))); } module.exports = { deep };",
        "deep",
        false,
        &function,
    )
    .is_some());
}

#[test]
fn reorders_pure_subtraction_without_changing_operand_order() {
    let function = thaw_bridge::DtsFunction {
        param_field_constraints: Vec::new(),
        name: "deep".into(),
        generic: None,
        params: vec![(
            "flag".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        )],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    let program = jit_numeric_export(
        "function deep(flag) { return 1000 - (900 - (800 - (700 - (600 - (500 - (400 - (300 - (flag ? 200 : 100)))))))); } module.exports = { deep };",
        "deep",
        false,
        &function,
    )
    .expect("pure right-nested subtraction should fit JIT registers");
    assert!(program.contains("rsub"), "expected reverse subtraction: {program}");
}
