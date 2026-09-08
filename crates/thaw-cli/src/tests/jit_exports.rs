#[test]
fn recognizes_only_pure_binary_numeric_commonjs_exports_for_jit() {
    let function = thaw_bridge::DtsFunction {
        name: "add".into(),
        generic: None,
        params: vec![
            (
                "left".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "right".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };

    assert_eq!(
        jit_numeric_export(
            "module.exports.add = function(left, right) { const sum = left + right; const doubled = sum * 2; const delta = left - right; if (left < right) return doubled; return delta; };",
            "add",
            false,
            &function,
        ),
        Some(
            "expr:a0,a1,<,if,a0,a1,+,c4000000000000000,*,else,a0,a1,-,end".into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = function(left, right) { let total = left; total += right; total *= 2; total--; return total; };",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+,c4000000000000000,*,c3ff0000000000000,-".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = function(left, right) { if (left) { if (right < 0) return 1; return 2; } else if (right) return 3; else return 4; };",
            "add",
            false,
            &function,
        ),
        Some(
            "expr:a0,asbool,a1,c0000000000000000,<,c3ff0000000000000,c4000000000000000,?,a1,asbool,c4008000000000000,c4010000000000000,?,?".into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports = { add: (left, right) => left + right, sub: (left, right) => left - right };",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "'use strict'; exports.sub = (left, right) => left - right; exports.add = (left, right) => left + right;",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const SCALE = 2, OFFSET = 1; exports.add = (left, right) => (left + right) * SCALE + OFFSET;",
            "add",
            false,
            &function,
        ),
        Some(
            "expr:a0,a1,+,c4000000000000000,*,c3ff0000000000000,+".into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "function add(left, right) { return left + right; } module.exports = { add };",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "function sum(left, right) { return left + right; } module.exports = { add: sum };",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const alias = double; module.exports.add = (left, right) => alias(sum(left, right)); function sum(left, right) { return left + right; } function double(value) { return value * 2; }",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+,c4000000000000000,*".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => normalize(left + right); function normalize(value) { const rounded = Math.round(value); if (rounded > 0) return rounded; return 0; }",
            "add",
            false,
            &function,
        ),
        Some(
            "expr:a0,a1,+,round,c0000000000000000,>,if,a0,a1,+,round,else,c0000000000000000,end"
                .into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "function add(left, right) { return right <= 0 ? left : add(left + 1, right - 1); } module.exports.add = add;",
            "add",
            false,
            &function,
        ),
        Some(
            "expr:a1,c0000000000000000,<=,if,a0,else,a0,c3ff0000000000000,+,a1,c3ff0000000000000,-,recurn2,end"
                .into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "function helper(value) { return value * 2; } module.exports.add = (helper, right) => helper(right);",
            "add",
            false,
            &function,
        ),
        None
    );
    for (source, allow_default) in [
        (
            "function add(left, right) { return left + right; } exports.add = add;",
            false,
        ),
        (
            "function add(left, right) { return left + right; } module.exports = add;",
            true,
        ),
    ] {
        assert_eq!(
            jit_numeric_export(source, "add", allow_default, &function),
            Some("expr:a0,a1,+".into())
        );
    }
    assert_eq!(
        jit_numeric_export(
            "const sum = (left, right) => left + right, add = sum; module.exports = { add };",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const left = 99; exports.add = (left, right) => left + right;",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.abs(left % right);",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,%,abs".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.max(left, right, 42);",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,max,c4045000000000000,max".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.floor(Math.sqrt(left));",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,sqrt,floor".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.log2(Math.exp(left));",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,exp,log2".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.atan2(left, right);",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,atan2".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.hypot(left, right, 12);",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,hypot,c4028000000000000,hypot".into())
    );
    let spread_extreme = thaw_bridge::DtsFunction {
        params: vec![(
            "values".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                thaw_hir::HirType::F64,
            ))),
        )],
        required_params: 1,
        ..function.clone()
    };
    let quantifier = thaw_bridge::DtsFunction {
        params: vec![
            spread_extreme.params[0].clone(),
            (
                "threshold".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ..spread_extreme.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, threshold) => values.some(value => value > threshold);",
            "add",
            false,
            &quantifier,
        ),
        Some("expr:rn0,a1,rnsomegt".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, threshold) => values.every(value => threshold <= value);",
            "add",
            false,
            &quantifier,
        ),
        Some("expr:rn0,a1,rneverygte".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const positive = value => value > 0; module.exports.add = values => values.some(positive);",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..quantifier.clone()
            },
        ),
        Some("expr:rn0,c0000000000000000,rnsomegt".into())
    );
    let string_quantifier = thaw_bridge::DtsFunction {
        params: vec![
            spread_extreme.params[0].clone(),
            (
                "expected".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
        ],
        ..quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, expected) => values.some(value => value == expected);",
            "add",
            false,
            &string_quantifier,
        ),
        Some("expr:rn0,s1,strnum,rnsomeeq".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, expected) => values.some(value => value === expected);",
            "add",
            false,
            &string_quantifier,
        ),
        Some(
            "expr:rn0,t61302c73332c73747269637466616c7365,arrayempty,s1,captureappend,rnsomejitc"
                .into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.some(value => value > Math.random());",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..quantifier.clone()
            },
        ),
        None
    );
    let finder = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, threshold) => values.find(value => value > threshold);",
            "add",
            false,
            &finder,
        ),
        Some("expr:rn0,a1,rnfindgt".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, threshold) => values.findLast(value => threshold <= value);",
            "add",
            false,
            &finder,
        ),
        Some("expr:rn0,a1,rnfindlastgte".into())
    );
    let finder_index = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, threshold) => values.findIndex(value => value === threshold);",
            "add",
            false,
            &finder_index,
        ),
        Some("expr:rn0,a1,rnfindindexeq".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, threshold) => values.findLastIndex(value => value !== threshold);",
            "add",
            false,
            &finder_index,
        ),
        Some("expr:rn0,a1,rnfindlastindexne".into())
    );
    let filter = thaw_bridge::DtsFunction {
        ret: spread_extreme.params[0].1.clone(),
        ..quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, threshold) => values.filter(value => value >= threshold);",
            "add",
            false,
            &filter,
        ),
        Some("expr:rn0,a1,rnfiltergte,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, factor) => values.map(value => { const scaled = value * factor; return scaled; });",
            "add",
            false,
            &filter,
        ),
        Some("expr:rn0,a1,rnmapmul,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, factor) => values.map(value => { let scaled = value; scaled *= factor; return scaled; });",
            "add",
            false,
            &filter,
        ),
        Some("expr:rn0,a1,rnmapmul,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, minimum) => values.map(value => value >= minimum ? value : minimum);",
            "add",
            false,
            &filter,
        ),
        Some("expr:rn0,a1,rnmapselectgte0,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, pivot) => values.map(value => value >= pivot ? value - pivot : pivot - value);",
            "add",
            false,
            &filter,
        ),
        Some("expr:rn0,a1,rnmapbranch933,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const positive = value => value > 0; module.exports.add = values => values.filter(positive);",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..filter.clone()
            },
        ),
        Some("expr:rn0,c0000000000000000,rnfiltergt,arrayvalue".into())
    );
    for (body, operation) in [("value + index", "add"), ("index - value", "rsub")] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.add = values => values.map((value, index) => {body});"),
                "add",
                false,
                &thaw_bridge::DtsFunction {
                    params: vec![spread_extreme.params[0].clone()],
                    required_params: 1,
                    ..filter.clone()
                },
            ),
            Some(format!("expr:rn0,rnmapindex{operation},arrayvalue")),
            "{body}"
        );
    }
    let callback = "a0,a0,*,c3ff0000000000000,+";
    let encoded_callback = callback
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.map(value => value * value + 1);",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..filter.clone()
            },
        ),
        Some(format!("expr:rn0,t{encoded_callback},rnmapjit,arrayvalue"))
    );
    let unary_quantifier = thaw_bridge::DtsFunction {
        params: vec![spread_extreme.params[0].clone()],
        required_params: 1,
        ..quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.some(Boolean);",
            "add",
            false,
            &unary_quantifier,
        ),
        Some("expr:rn0,rnsometruthy".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.every(value => !!value);",
            "add",
            false,
            &unary_quantifier,
        ),
        Some("expr:rn0,rneverytruthy".into())
    );
    for (method, suffix, function) in [
        ("find", "find", &finder),
        ("findLast", "findlast", &finder),
        ("findIndex", "findindex", &finder_index),
        ("findLastIndex", "findlastindex", &finder_index),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.add = values => values.{method}(value => value);"),
                "add",
                false,
                &thaw_bridge::DtsFunction {
                    params: vec![spread_extreme.params[0].clone()],
                    required_params: 1,
                    ..function.clone()
                },
            ),
            Some(format!("expr:rn0,rn{suffix}truthy")),
            "{method}"
        );
    }
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.filter(Boolean);",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..filter.clone()
            },
        ),
        Some("expr:rn0,rnfiltertruthy,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const Boolean = value => false; module.exports.add = values => values.filter(Boolean);",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..filter.clone()
            },
        ),
        Some("expr:rn0,t6330303030303030303030303030303030,rnfilterjit,arrayvalue".into())
    );
    let string_array =
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Str)));
    let string_filter = thaw_bridge::DtsFunction {
        params: vec![("values".into(), string_array.clone())],
        required_params: 1,
        ret: string_array,
        ..filter.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.filter(Boolean);",
            "add",
            false,
            &string_filter,
        ),
        Some("expr:rs0,rsfiltertruthy,arrayvalue".into())
    );
    let string_finder = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::Str,
        ))),
        ..string_filter.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.find(value => value);",
            "add",
            false,
            &string_finder,
        ),
        Some("expr:rs0,rsfindtruthy".into())
    );
    let bool_array =
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Bool)));
    let bool_quantifier = thaw_bridge::DtsFunction {
        params: vec![("values".into(), bool_array.clone())],
        required_params: 1,
        ..unary_quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.some(Boolean);",
            "add",
            false,
            &bool_quantifier,
        ),
        Some("expr:rb0,rbsometruthy".into())
    );
    let compared_string_filter = thaw_bridge::DtsFunction {
        params: vec![
            string_filter.params[0].clone(),
            (
                "minimum".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
        ],
        required_params: 2,
        ..string_filter.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, minimum) => values.filter(value => value >= minimum);",
            "add",
            false,
            &compared_string_filter,
        ),
        Some("expr:rs0,s1,rsfiltergte,arrayvalue".into())
    );
    let compared_bool_quantifier = thaw_bridge::DtsFunction {
        params: vec![
            bool_quantifier.params[0].clone(),
            (
                "expected".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
            ),
        ],
        required_params: 2,
        ..bool_quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, expected) => values.some(value => value === expected);",
            "add",
            false,
            &compared_bool_quantifier,
        ),
        Some("expr:rb0,b1,rbsomeeq".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.map(value => value);",
            "add",
            false,
            &string_filter,
        ),
        Some("expr:rs0,rsmapidentity,arrayvalue".into())
    );
    for (method, operation) in [
        ("toLowerCase", "tolowercase"),
        ("toUpperCase", "touppercase"),
        ("trim", "trim"),
        ("trimStart", "trimstart"),
        ("trimEnd", "trimend"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.add = values => values.map(value => value.{method}());"),
                "add",
                false,
                &string_filter,
            ),
            Some(format!("expr:rs0,rsmap{operation},arrayvalue")),
            "{method}"
        );
    }
    let string_lengths = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..string_filter.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.map(value => value.length);",
            "add",
            false,
            &string_lengths,
        ),
        Some("expr:rs0,rsmaplength,arrayvalue".into())
    );
    for (callback, operation, result) in [
        ("String", "string", string_filter.ret.clone()),
        (
            "Number",
            "number",
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                thaw_hir::HirType::F64,
            ))),
        ),
        (
            "Boolean",
            "boolean",
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                thaw_hir::HirType::Bool,
            ))),
        ),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.add = values => values.map({callback});"),
                "add",
                false,
                &thaw_bridge::DtsFunction {
                    ret: result,
                    ..string_filter.clone()
                },
            ),
            Some(format!("expr:rs0,rsmapto{operation},arrayvalue")),
            "{callback}"
        );
    }
    assert_eq!(
        jit_numeric_export(
            "const String = value => value; module.exports.add = values => values.map(String);",
            "add",
            false,
            &string_filter,
        ),
        Some("expr:rs0,rsmapidentity,arrayvalue".into())
    );
    let bool_map = thaw_bridge::DtsFunction {
        ret: bool_array.clone(),
        ..bool_quantifier.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.map(value => !value);",
            "add",
            false,
            &bool_map,
        ),
        Some("expr:rb0,rbmapnot,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.filter(value => value === 1);",
            "add",
            false,
            &string_filter,
        ),
        Some(
            "expr:rs0,t73302c63336666303030303030303030303030302c73747269637466616c7365,rsfilterjit,arrayvalue"
                .into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.find(Boolean);",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                    thaw_hir::HirType::Bool,
                ))),
                ..bool_quantifier.clone()
            },
        ),
        Some("expr:rb0,rbfindtruthy".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, factor) => values.map(value => value * factor);",
            "add",
            false,
            &filter,
        ),
        Some("expr:rn0,a1,rnmapmul,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const subtract = value => 100 - value; module.exports.add = values => values.map(subtract);",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..filter.clone()
            },
        ),
        Some("expr:rn0,c4059000000000000,rnmaprsub,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.map(value => value + Math.random());",
            "add",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![spread_extreme.params[0].clone()],
                required_params: 1,
                ..filter.clone()
            },
        ),
        None
    );
    let unary_map = thaw_bridge::DtsFunction {
        params: vec![spread_extreme.params[0].clone()],
        required_params: 1,
        ..filter.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.map(value => -value);",
            "add",
            false,
            &unary_map,
        ),
        Some("expr:rn0,rnmapneg,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const absolute = value => Math.abs(value); module.exports.add = values => values.map(absolute);",
            "add",
            false,
            &unary_map,
        ),
        Some("expr:rn0,rnmapabs,arrayvalue".into())
    );
    for method in [
        "acos", "acosh", "asin", "asinh", "atan", "atanh", "cbrt", "ceil", "clz32", "cos", "cosh",
        "exp", "expm1", "floor", "fround", "log", "log1p", "log2", "log10", "round", "sign", "sin",
        "sinh", "sqrt", "tan", "tanh", "trunc",
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!(
                    "module.exports.add = values => values.map(value => Math.{method}(value));"
                ),
                "add",
                false,
                &unary_map,
            ),
            Some(format!("expr:rn0,rnmap{method},arrayvalue")),
            "{method}"
        );
    }
    assert_eq!(
        jit_numeric_export(
            "const Math = { abs: value => value }; module.exports.add = values => values.map(value => Math.abs(value));",
            "add",
            false,
            &unary_map,
        ),
        None
    );
    for (method, operation) in [("min", "rnmin"), ("max", "rnmax")] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.add = values => Math.{method}(...values);"),
                "add",
                false,
                &spread_extreme,
            ),
            Some(format!("expr:rn0,{operation}"))
        );
    }
    let mixed_extreme = thaw_bridge::DtsFunction {
        params: vec![
            (
                "left".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            spread_extreme.params[0].clone(),
            ("tail".into(), spread_extreme.params[0].1.clone()),
            (
                "right".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 4,
        ..spread_extreme.clone()
    };
    for (method, operation) in [("min", "rnmin"), ("max", "rnmax")] {
        assert_eq!(
            jit_numeric_export(
                &format!(
                    "module.exports.add = (left, values, tail, right) => Math.{method}(left, ...values, ...tail, right);"
                ),
                "add",
                false,
                &mixed_extreme,
            ),
            Some(format!(
                "expr:a0,rn1,{operation},{method},rn2,{operation},{method},a3,{method}"
            ))
        );
    }
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => Math.hypot(...values);",
            "add",
            false,
            &spread_extreme,
        ),
        Some("expr:rn0,rnhypot".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.reduce(function(accumulator, value) { return accumulator + value; }, 5);",
            "add",
            false,
            &spread_extreme,
        ),
        Some("expr:rn0,c4014000000000000,rnreduceadd".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const addValues = (accumulator, value) => { const total = accumulator + value; return total; }; module.exports.add = values => values.reduce(addValues, 0);",
            "add",
            false,
            &spread_extreme,
        ),
        Some("expr:rn0,c0000000000000000,rnreduceadd".into())
    );
    assert_eq!(
        jit_numeric_export(
            "function combine(accumulator, value) { return accumulator + value; } module.exports.add = values => values.reduce((accumulator, value) => combine(accumulator, value), 0);",
            "add",
            false,
            &spread_extreme,
        ),
        Some("expr:rn0,c0000000000000000,rnreduceadd".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.reduce((accumulator, value) => { const ignored = Math.random(); return accumulator + value; }, 0);",
            "add",
            false,
            &spread_extreme,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.reduce(addValues, 0); function addValues(accumulator, value) { return accumulator + value; }",
            "add",
            false,
            &spread_extreme,
        ),
        Some("expr:rn0,c0000000000000000,rnreduceadd".into())
    );
    for (operator, operation) in [
        ("+", "add"),
        ("-", "sub"),
        ("*", "mul"),
        ("/", "div"),
        ("%", "rem"),
        ("**", "pow"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!(
                    "module.exports.add = values => values.reduce((accumulator, value) => accumulator {operator} value, 0);"
                ),
                "add",
                false,
                &spread_extreme,
            ),
            Some(format!(
                "expr:rn0,c0000000000000000,rnreduce{operation}"
            ))
        );
        assert_eq!(
            jit_numeric_export(
                &format!(
                    "module.exports.add = values => values.reduce((accumulator, value) => accumulator {operator} value);"
                ),
                "add",
                false,
                &spread_extreme,
            ),
            Some(format!(
                "expr:rn0,c0000000000000000,rnreduce{operation}0"
            ))
        );
        for (initial, suffix) in [(true, ""), (false, "0")] {
            assert_eq!(
                jit_numeric_export(
                    &format!(
                        "module.exports.add = values => values.reduceRight((accumulator, value) => accumulator {operator} value{});",
                        if initial { ", 0" } else { "" }
                    ),
                    "add",
                    false,
                    &spread_extreme,
                ),
                Some(format!(
                    "expr:rn0,c0000000000000000,rnreduceright{operation}{suffix}"
                ))
            );
        }
    }
    for (method, direction) in [("min", "reduce"), ("max", "reduceRight")] {
        for (initial, suffix) in [(true, ""), (false, "0")] {
            assert_eq!(
                jit_numeric_export(
                    &format!(
                        "module.exports.add = values => values.{direction}((accumulator, value) => Math.{method}(accumulator, value){});",
                        if initial { ", 0" } else { "" }
                    ),
                    "add",
                    false,
                    &spread_extreme,
                ),
                Some(format!(
                    "expr:rn0,c0000000000000000,rnreduce{}{method}{suffix}",
                    if direction == "reduceRight" { "right" } else { "" }
                ))
            );
        }
    }
    let shadowed_math = thaw_bridge::DtsFunction {
        params: vec![
            spread_extreme.params[0].clone(),
            (
                "Math".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ..spread_extreme.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (values, Math) => values.reduce((accumulator, value) => Math.min(accumulator, value), 0);",
            "add",
            false,
            &shadowed_math,
        ),
        None
    );
    let shadowed_reducer = thaw_bridge::DtsFunction {
        params: vec![
            spread_extreme.params[0].clone(),
            (
                "addValues".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ..spread_extreme.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "const addValues = (accumulator, value) => accumulator + value; module.exports.add = (values, addValues) => values.reduce(addValues, 0);",
            "add",
            false,
            &shadowed_reducer,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = values => values.reduce((accumulator, value) => value + accumulator, 0);",
            "add",
            false,
            &spread_extreme,
        ),
        Some("expr:rn0,c0000000000000000,t61312c61302c2b,rnreducejit".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.hypot(left);",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,abs".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, values, tail, right) => Math.hypot(left, ...values, ...tail, right);",
            "add",
            false,
            &mixed_extreme,
        ),
        Some("expr:a0,rn1,rnhypot,hypot,rn2,rnhypot,hypot,a3,hypot".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.PI * left + Math.E;",
            "add",
            false,
            &function,
        ),
        Some(format!(
            "expr:c{:016x},a0,*,c{:016x},+",
            std::f64::consts::PI.to_bits(),
            std::f64::consts::E.to_bits()
        ))
    );
    for (expression, value) in [
        ("Number.EPSILON", f64::EPSILON),
        ("Number.MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
        ("Number.MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
        ("Number.MAX_VALUE", f64::MAX),
        ("Number.MIN_VALUE", f64::from_bits(1)),
        ("Number.NaN", f64::NAN),
        ("Number.POSITIVE_INFINITY", f64::INFINITY),
        ("Number.NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("NaN", f64::NAN),
        ("Infinity", f64::INFINITY),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.add = (left, right) => {expression};"),
                "add",
                false,
                &function,
            ),
            Some(format!("expr:c{:016x}", value.to_bits()))
        );
    }
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (Number, right) => Number.MAX_VALUE;",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (Infinity, right) => Infinity;",
            "add",
            false,
            &function,
        ),
        Some("expr:a0".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => left + Math.sin(right);",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,sin,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => Math.imul(Math.clz32(left), Math.fround(right));",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,clz32,a1,fround,imul".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (Math, right) => Math.PI;",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => ((left & 255) ^ right) >>> 0;",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,c406fe00000000000,band,a1,bxor,c0000000000000000,ushr".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (left, right) => (left || right) + (left && right);",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,dup,asbool,||,a1,end,a0,dup,asbool,&&,a1,end,+".into())
    );
    for source in [
        "module.exports.add = (left, right) => left ** right;",
        "module.exports.add = (left, right) => Math.pow(left, right);",
    ] {
        assert_eq!(
            jit_numeric_export(source, "add", false, &function),
            Some("expr:a0,a1,pow".into())
        );
    }
    let mut predicate = function.clone();
    predicate.name = "less".into();
    predicate.ret = thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool);
    assert_eq!(
        jit_numeric_export(
            "module.exports.less = (left, right) => left < right;",
            "less",
            false,
            &predicate,
        ),
        Some("expr:a0,a1,<".into())
    );
    predicate.params.truncate(1);
    predicate.required_params = 1;
    for (name, source, operation) in [
        ("finite", "value => Number.isFinite(value)", "isfinite"),
        ("integer", "value => Number.isInteger(value)", "isinteger"),
        (
            "safeInteger",
            "value => Number.isSafeInteger(value)",
            "issafeinteger",
        ),
        ("nan", "value => Number.isNaN(value)", "isnan"),
        ("globalFinite", "value => isFinite(value)", "isfinite"),
        ("globalNan", "value => isNaN(value)", "isnan"),
    ] {
        predicate.name = name.into();
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.{name} = {source};"),
                name,
                false,
                &predicate,
            ),
            Some(format!("expr:a0,{operation}"))
        );
    }
    predicate.name = "shadowedNan".into();
    assert_eq!(
        jit_numeric_export(
            "const isNaN = value => false; module.exports.shadowedNan = value => isNaN(value);",
            "shadowedNan",
            false,
            &predicate,
        ),
        Some("expr:c0000000000000000".into())
    );
    predicate.name = "stringNan".into();
    predicate.params[0].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::Str);
    assert_eq!(
        jit_numeric_export(
            "module.exports.stringNan = value => Number.isNaN(value);",
            "stringNan",
            false,
            &predicate,
        ),
        Some("expr:s0,c0000000000000000,strictfalse".into())
    );
    predicate.name = "negateFlag".into();
    predicate.params = vec![(
        "value".into(),
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
    )];
    predicate.required_params = 1;
    assert_eq!(
        jit_numeric_export(
            "module.exports.negateFlag = value => !value;",
            "negateFlag",
            false,
            &predicate,
        ),
        Some("expr:b0,boolnot".into())
    );
    let string_length = thaw_bridge::DtsFunction {
        name: "length".into(),
        generic: None,
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        )],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = value => value.length;",
            "length",
            false,
            &string_length,
        ),
        Some("expr:s0,strlen".into())
    );
    for (source, expected) in [
        (
            "module.exports.length = value => Number(value);",
            "expr:s0,strnum",
        ),
        ("module.exports.length = value => +value;", "expr:s0,strnum"),
        (
            "module.exports.length = value => value * '2';",
            "expr:s0,strnum,t32,strnum,*",
        ),
        (
            "module.exports.length = value => Math.round(value);",
            "expr:s0,strnum,round",
        ),
    ] {
        assert_eq!(
            jit_numeric_export(source, "length", false, &string_length),
            Some(expected.into())
        );
    }
    let number_format = thaw_bridge::DtsFunction {
        name: "format".into(),
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "argument".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ..string_length.clone()
    };
    for (method, operation) in [
        ("toFixed", "tofixed"),
        ("toPrecision", "toprecision"),
        ("toString", "toradix"),
        ("toExponential", "toexponential"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.format = (value, argument) => value.{method}(argument);"),
                "format",
                false,
                &number_format,
            ),
            Some(format!("expr:a0,a1,{operation}"))
        );
    }
    let number_to_string = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        )],
        required_params: 1,
        ..number_format.clone()
    };
    for (method, expected) in [
        ("toFixed", "expr:a0,c0000000000000000,tofixed"),
        ("toPrecision", "expr:a0,numstr"),
        ("toString", "expr:a0,numstr"),
        ("toExponential", "expr:a0,toexponential0"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.format = value => value.{method}();"),
                "format",
                false,
                &number_to_string,
            ),
            Some(expected.into())
        );
    }
    let boolean_to_string = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        )],
        ..number_to_string.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.format = value => value.toString();",
            "format",
            false,
            &boolean_to_string,
        ),
        Some("expr:b0,boolstr".into())
    );
    let string_value = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        )],
        ..number_to_string.clone()
    };
    for (source, expected) in [
        ("value.toString()", "expr:s0"),
        ("value.valueOf()", "expr:s0"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.format = value => {source};"),
                "format",
                false,
                &string_value,
            ),
            Some(expected.into())
        );
    }
    let boolean_value = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ..boolean_to_string.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.format = value => value.valueOf();",
            "format",
            false,
            &boolean_value,
        ),
        Some("expr:b0".into())
    );
    let number_value = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..number_to_string.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.format = value => value.valueOf();",
            "format",
            false,
            &number_value,
        ),
        Some("expr:a0".into())
    );
    let array_length = thaw_bridge::DtsFunction {
        params: vec![(
            "values".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                thaw_hir::HirType::F64,
            ))),
        )],
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..number_to_string.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = values => values.length;",
            "length",
            false,
            &array_length,
        ),
        Some("expr:rn0,arraylen".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = values => values + 1;",
            "length",
            false,
            &array_length,
        ),
        None
    );
    let array_return = thaw_bridge::DtsFunction {
        params: Vec::new(),
        required_params: 0,
        ret: array_length.params[0].1.clone(),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.values = () => [1, 2];",
            "values",
            false,
            &array_return,
        ),
        Some(
            "expr:arrayempty,c3ff0000000000000,rnappend,c4000000000000000,rnappend,arrayvalue"
                .into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.values = () => [];",
            "values",
            false,
            &array_return,
        ),
        Some("expr:arrayempty,arrayvalue".into())
    );
    let spread_array = thaw_bridge::DtsFunction {
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            ("tail".into(), array_length.params[0].1.clone()),
        ],
        required_params: 2,
        ..array_return.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.values = (value, tail) => [1, value, ...tail];",
            "values",
            false,
            &spread_array,
        ),
        Some(
            "expr:arrayempty,c3ff0000000000000,rnappend,a0,rnappend,rn1,arrayconcat,arrayvalue"
                .into()
        )
    );
    let spread_strings = thaw_bridge::DtsFunction {
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "tail".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                    thaw_hir::HirType::Str,
                ))),
            ),
        ],
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
            thaw_hir::HirType::Str,
        ))),
        ..spread_array.clone()
    };
    assert!(jit_numeric_export(
        "module.exports.values = (value, tail) => ['a', value, ...tail];",
        "values",
        false,
        &spread_strings,
    )
    .is_some());
    let string_spread = thaw_bridge::DtsFunction {
        params: vec![spread_strings.params[0].clone()],
        required_params: 1,
        ..spread_strings.clone()
    };
    for source in [
        "module.exports.values = value => [...value];",
        "module.exports.values = value => Array.of(...value);",
    ] {
        assert_eq!(
            jit_numeric_export(source, "values", false, &string_spread),
            Some("expr:arrayempty,s0,strarray,arrayconcat,arrayvalue".into())
        );
    }
    let spread_flags = thaw_bridge::DtsFunction {
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
            ),
            (
                "tail".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                    thaw_hir::HirType::Bool,
                ))),
            ),
        ],
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
            thaw_hir::HirType::Bool,
        ))),
        ..spread_array.clone()
    };
    assert!(jit_numeric_export(
        "module.exports.values = (value, tail) => [true, value, ...tail];",
        "values",
        false,
        &spread_flags,
    )
    .is_some());
    assert_eq!(
        jit_numeric_export(
            "module.exports.values = (value, tail) => Array.of(1, value, ...tail);",
            "values",
            false,
            &spread_array,
        ),
        Some(
            "expr:arrayempty,c3ff0000000000000,rnappend,a0,rnappend,rn1,arrayconcat,arrayvalue"
                .into()
        )
    );
    let copy_array = thaw_bridge::DtsFunction {
        params: vec![("values".into(), array_length.params[0].1.clone())],
        required_params: 1,
        ..array_return.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.values = values => Array.from(values);",
            "values",
            false,
            &copy_array,
        ),
        Some("expr:rn0,c0000000000000000,c7ff0000000000000,arrayslice,arrayvalue".into())
    );
    let shadowed_array_constructor = thaw_bridge::DtsFunction {
        params: vec![("Array".into(), array_length.params[0].1.clone())],
        ..array_return
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.values = Array => Array.from(Array);",
            "values",
            false,
            &shadowed_array_constructor,
        ),
        None
    );
    let array_predicate = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.check = values => Array.isArray(values);",
            "check",
            false,
            &array_predicate,
        ),
        Some("expr:rn0,isarray".into())
    );
    let number_predicate = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        )],
        ..array_predicate.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.check = value => Array.isArray(value);",
            "check",
            false,
            &number_predicate,
        ),
        Some("expr:a0,isnotarray".into())
    );
    let shadowed_array = thaw_bridge::DtsFunction {
        params: vec![("Array".into(), array_length.params[0].1.clone())],
        ..array_predicate.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.check = Array => Array.isArray(Array);",
            "check",
            false,
            &shadowed_array,
        ),
        None
    );
    let same_number = thaw_bridge::DtsFunction {
        required_params: 2,
        params: vec![
            (
                "left".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "right".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        ..array_predicate.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.check = (left, right) => Object.is(left, right);",
            "check",
            false,
            &same_number,
        ),
        Some("expr:a0,a1,numsame".into())
    );
    let shadowed_object = thaw_bridge::DtsFunction {
        params: vec![
            (
                "Object".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            same_number.params[1].clone(),
        ],
        ..same_number
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.check = (Object, right) => Object.is(Object, right);",
            "check",
            false,
            &shadowed_object,
        ),
        None
    );
    let type_of_number = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        required_params: 1,
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        )],
        ..array_predicate.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.kind = value => typeof value;",
            "kind",
            false,
            &type_of_number,
        ),
        Some("expr:a0,typeofnumber".into())
    );
    let zero_arg_number = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        required_params: 0,
        params: Vec::new(),
        ..array_predicate.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.empty = () => Number();",
            "empty",
            false,
            &zero_arg_number,
        ),
        Some("expr:c0000000000000000".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.random = () => 1 + Math.random();",
            "random",
            false,
            &zero_arg_number,
        ),
        Some("expr:c3ff0000000000000,random,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.now = () => Date.now() + performance.now();",
            "now",
            false,
            &zero_arg_number,
        ),
        Some("expr:datenow,performancenow,+".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.uptime = () => process.uptime();",
            "uptime",
            false,
            &zero_arg_number,
        ),
        Some("expr:performancenow,c408f400000000000,/".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.pid = () => process.pid;",
            "pid",
            false,
            &zero_arg_number,
        ),
        Some("expr:processpid".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.ppid = () => process.ppid;",
            "ppid",
            false,
            &zero_arg_number,
        ),
        Some("expr:processppid".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.pid = process => process.pid;",
            "pid",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![(
                    "process".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                )],
                required_params: 1,
                ..zero_arg_number.clone()
            },
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.now = Date => Date.now();",
            "now",
            false,
            &thaw_bridge::DtsFunction {
                params: vec![(
                    "Date".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                )],
                required_params: 1,
                ..zero_arg_number.clone()
            },
        ),
        None
    );
    let zero_arg_string = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ..zero_arg_number.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.empty = () => String();",
            "empty",
            false,
            &zero_arg_string,
        ),
        Some("expr:t".into())
    );
    let string_codes = thaw_bridge::DtsFunction {
        params: vec![
            (
                "left".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "right".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ..zero_arg_string.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.text = (left, right) => String.fromCharCode(left, right);",
            "text",
            false,
            &string_codes,
        ),
        Some("expr:a0,fromcharcode,a1,fromcharcode,concat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.text = (left, right) => String.fromCodePoint(left, right);",
            "text",
            false,
            &string_codes,
        ),
        Some("expr:a0,fromcodepoint,a1,fromcodepoint,concat".into())
    );
    let zero_arg_boolean = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ..zero_arg_number
    };
    assert!(jit_numeric_export(
        "module.exports.constants = () => Number.MIN_VALUE > 0;",
        "constants",
        false,
        &zero_arg_boolean,
    )
    .is_some());
    assert!(jit_numeric_export(
        "module.exports.globals = () => Number.isNaN(NaN) && !isFinite(Infinity);",
        "globals",
        false,
        &zero_arg_boolean,
    )
    .is_some());
    assert_eq!(
        jit_numeric_export(
            "module.exports.empty = () => Boolean();",
            "empty",
            false,
            &zero_arg_boolean,
        ),
        Some("expr:c0000000000000000".into())
    );
    let defaulted_numbers = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        required_params: 0,
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                    thaw_hir::HirType::F64,
                ))),
            ),
            (
                "factor".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                    thaw_hir::HirType::F64,
                ))),
            ),
        ],
        ..array_predicate.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.scale = (value = 2, factor = value + 1) => value * factor;",
            "scale",
            false,
            &defaulted_numbers,
        ),
        Some(
            "expr:a0,asbool,if,a1,else,c4000000000000000,end,a2,asbool,if,a3,else,a0,asbool,if,a1,else,c4000000000000000,end,c3ff0000000000000,+,end,*"
                .into()
        )
    );
    let optional_number = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                thaw_hir::HirType::F64,
            ))),
        )],
        required_params: 0,
        ..defaulted_numbers.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.value = value => value ?? 42;",
            "value",
            false,
            &optional_number,
        ),
        Some("expr:a0,asbool,if,a1,else,c4045000000000000,end".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.value = value => value + 1;",
            "value",
            false,
            &optional_number,
        ),
        None
    );
    let optional_string_length = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                thaw_hir::HirType::Str,
            ))),
        )],
        required_params: 0,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..optional_number.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = value => value?.length;",
            "length",
            false,
            &optional_string_length,
        ),
        Some("expr:a0,asbool,if,s1,strlen,else,absentn,end".into())
    );
    let coalesced_string_length = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..optional_string_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = value => value?.length ?? 42;",
            "length",
            false,
            &coalesced_string_length,
        ),
        Some("expr:a0,asbool,if,s1,strlen,else,c4045000000000000,end".into())
    );
    let optional_upper = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ..optional_string_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.upper = value => value?.toUpperCase() ?? 'missing';",
            "upper",
            false,
            &optional_upper,
        ),
        Some("expr:a0,asbool,if,s1,touppercase,else,t6d697373696e67,end".into())
    );
    let optional_object_number = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                thaw_hir::HirType::Object(vec![
                    (
                        "nested".into(),
                        thaw_hir::HirType::Object(vec![("score".into(), thaw_hir::HirType::F64)]),
                    ),
                    (
                        "pair".into(),
                        thaw_hir::HirType::Tuple(vec![
                            thaw_hir::HirType::F64,
                            thaw_hir::HirType::Str,
                        ]),
                    ),
                    (
                        "flags".into(),
                        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Bool)),
                    ),
                    ("label".into(), thaw_hir::HirType::Str),
                ]),
            ))),
        )],
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..optional_number.clone()
    };
    assert!(jit_numeric_export(
        "module.exports.score = value => value?.nested.score;",
        "score",
        false,
        &optional_object_number,
    )
    .is_some());
    let optional_object_string = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::Str,
        ))),
        ..optional_object_number.clone()
    };
    assert!(jit_numeric_export(
        "module.exports.name = value => value?.pair[1];",
        "name",
        false,
        &optional_object_string,
    )
    .is_some());
    let object_array_length = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..optional_object_number.clone()
    };
    assert!(jit_numeric_export(
        "module.exports.length = value => value?.flags.length ?? 42;",
        "length",
        false,
        &object_array_length,
    )
    .is_some());
    let object_upper = thaw_bridge::DtsFunction {
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ..optional_object_number.clone()
    };
    assert!(jit_numeric_export(
        "module.exports.upper = value => value?.label.toUpperCase() ?? 'missing';",
        "upper",
        false,
        &object_upper,
    )
    .is_some());
    let optional_push = thaw_bridge::DtsFunction {
        params: vec![
            (
                "source".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                    thaw_hir::HirType::F64,
                ))),
            ),
            (
                "values".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                    thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)),
                ))),
            ),
        ],
        required_params: 1,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..optional_upper
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.push = (source, values) => values?.push(source.pop()) ?? 0;",
            "push",
            false,
            &optional_push,
        ),
        Some("expr:a1,asbool,if,rn2,rn0,rnpop,rnpush,else,c0000000000000000,end".into())
    );
    let string_characters = thaw_bridge::DtsFunction {
        params: vec![(
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        )],
        required_params: 1,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
            thaw_hir::HirType::Str,
        ))),
        ..copy_array.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.values = value => Array.from(value);",
            "values",
            false,
            &string_characters,
        ),
        Some("expr:s0,strarray,arrayvalue".into())
    );
    let array_search = thaw_bridge::DtsFunction {
        params: vec![
            array_length.params[0].clone(),
            (
                "needle".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "from".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 3,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.has = (values, needle, from) => values.includes(needle, from);",
            "has",
            false,
            &array_search,
        ),
        Some("expr:rn0,a1,a2,rnincludes".into())
    );
    let mut array_index = array_search.clone();
    array_index.ret = thaw_bridge::DtsType::Native(thaw_hir::HirType::F64);
    assert_eq!(
        jit_numeric_export(
            "module.exports.find = (values, needle, from) => values.indexOf(needle, from);",
            "find",
            false,
            &array_index,
        ),
        Some("expr:rn0,a1,a2,rnindexof".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.find = (values, needle, from) => values.lastIndexOf(needle, from);",
            "find",
            false,
            &array_index,
        ),
        Some("expr:rn0,a1,a2,rnlastindexof".into())
    );
    let array_at = thaw_bridge::DtsFunction {
        params: vec![
            array_length.params[0].clone(),
            array_index.params[1].clone(),
        ],
        required_params: 2,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.pick = (values, needle) => values.at(needle);",
            "pick",
            false,
            &array_at,
        ),
        Some("expr:rn0,a1,rnat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.pick = (values, needle) => values[needle];",
            "pick",
            false,
            &array_at,
        ),
        Some("expr:rn0,a1,rnget".into())
    );
    let array_set = thaw_bridge::DtsFunction {
        params: vec![
            array_length.params[0].clone(),
            array_index.params[1].clone(),
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 3,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index, value) => values[index] = value;",
            "set",
            false,
            &array_set,
        ),
        Some("expr:rn0,a1,a2,rnset".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index, value) => values[index] += value;",
            "set",
            false,
            &array_set,
        ),
        Some("expr:rn0,a1,dup2,rnget,a2,+,rnset".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index) => ++values[index];",
            "set",
            false,
            &array_at,
        ),
        Some("expr:rn0,a1,dup2,rnget,c3ff0000000000000,+,rnset".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index) => values[index]--;",
            "set",
            false,
            &array_at,
        ),
        Some("expr:rn0,a1,dup2,rnget,dup,c3ff0000000000000,-,rnpostset".into())
    );
    let array_set_and_return = thaw_bridge::DtsFunction {
        ret: array_length.params[0].1.clone(),
        ..array_set.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index, value) => { values[index] = value; return values; };",
            "set",
            false,
            &array_set_and_return,
        ),
        Some("expr:rn0,a1,a2,rnset,drop,rn0,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index, value) => { values[index] *= value; return values; };",
            "set",
            false,
            &array_set_and_return,
        ),
        Some("expr:rn0,a1,dup2,rnget,a2,*,rnset,drop,rn0,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index) => { values[index]++; return values; };",
            "set",
            false,
            &thaw_bridge::DtsFunction {
                params: array_at.params.clone(),
                required_params: 2,
                ret: array_length.params[0].1.clone(),
                ..array_at.clone()
            },
        ),
        Some("expr:rn0,a1,dup2,rnget,dup,c3ff0000000000000,+,rnpostset,drop,rn0,arrayvalue".into())
    );
    let string_array_set = thaw_bridge::DtsFunction {
        params: vec![
            (
                "values".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                    thaw_hir::HirType::Str,
                ))),
            ),
            array_index.params[1].clone(),
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
        ],
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ..array_set.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.set = (values, index, value) => values[index] += value;",
            "set",
            false,
            &string_array_set,
        ),
        Some("expr:rs0,a1,dup2,rsget,s2,concat,rsset".into())
    );
    let array_join = thaw_bridge::DtsFunction {
        params: vec![
            array_length.params[0].clone(),
            (
                "separator".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
        ],
        required_params: 2,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.join = (values, separator) => values.join(separator);",
            "join",
            false,
            &array_join,
        ),
        Some("expr:rn0,s1,rnjoin".into())
    );
    let array_to_string = thaw_bridge::DtsFunction {
        params: vec![array_length.params[0].clone()],
        required_params: 1,
        ..array_join.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.join = values => values.toString();",
            "join",
            false,
            &array_to_string,
        ),
        Some("expr:rn0,t2c,rnjoin".into())
    );
    let array_slice = thaw_bridge::DtsFunction {
        name: "slice".into(),
        params: vec![
            array_length.params[0].clone(),
            (
                "start".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "end".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 3,
        ret: array_length.params[0].1.clone(),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.slice = (values, start, end) => values.slice(start, end);",
            "slice",
            false,
            &array_slice,
        ),
        Some("expr:rn0,a1,a2,arrayslice,arrayvalue".into())
    );
    let array_slice_all = thaw_bridge::DtsFunction {
        params: vec![array_length.params[0].clone()],
        required_params: 1,
        ..array_slice.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.slice = values => values.slice();",
            "slice",
            false,
            &array_slice_all,
        ),
        Some("expr:rn0,c0000000000000000,c7ff0000000000000,arrayslice,arrayvalue".into())
    );
    let array_reversed = thaw_bridge::DtsFunction {
        name: "reversed".into(),
        ..array_slice_all.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.reversed = values => values.toReversed();",
            "reversed",
            false,
            &array_reversed,
        ),
        Some("expr:rn0,arrayreversed,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.reversed = values => values.reverse();",
            "reversed",
            false,
            &array_reversed,
        ),
        Some("expr:rn0,arrayreverse,arrayvalue".into())
    );
    let array_sorted = thaw_bridge::DtsFunction {
        name: "sorted".into(),
        ..array_slice_all.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.sorted = values => values.toSorted();",
            "sorted",
            false,
            &array_sorted,
        ),
        Some("expr:rn0,rnsorted,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.sorted = values => values.sort();",
            "sorted",
            false,
            &array_sorted,
        ),
        Some("expr:rn0,rnsort,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.sorted = values => values.toSorted((left, right) => left - right);",
            "sorted",
            false,
            &array_sorted,
        ),
        Some("expr:rn0,rnsortedasc,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const descending = (left, right) => right - left; module.exports.sorted = values => values.sort(descending);",
            "sorted",
            false,
            &array_sorted,
        ),
        Some("expr:rn0,rnsortdesc,arrayvalue".into())
    );
    let string_array_sorted = thaw_bridge::DtsFunction {
        params: vec![(
            "values".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
                thaw_hir::HirType::Str,
            ))),
        )],
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
            thaw_hir::HirType::Str,
        ))),
        ..array_sorted.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.sorted = values => values.toSorted((left, right) => left.localeCompare(right));",
            "sorted",
            false,
            &string_array_sorted,
        ),
        Some("expr:rs0,rssorted,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "function descending(left, right) { return right.localeCompare(left); } module.exports.sorted = values => values.sort(descending);",
            "sorted",
            false,
            &string_array_sorted,
        ),
        Some("expr:rs0,rssortdesc,arrayvalue".into())
    );
    let array_fill = thaw_bridge::DtsFunction {
        name: "fill".into(),
        params: vec![
            array_length.params[0].clone(),
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "start".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "end".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 4,
        ret: array_length.params[0].1.clone(),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.fill = (values, value, start, end) => values.fill(value, start, end);",
            "fill",
            false,
            &array_fill,
        ),
        Some("expr:rn0,a1,a2,a3,rnfill,arrayvalue".into())
    );
    let array_copy_within = thaw_bridge::DtsFunction {
        name: "copy".into(),
        params: vec![
            array_length.params[0].clone(),
            (
                "target".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "start".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "end".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 4,
        ret: array_length.params[0].1.clone(),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.copy = (values, target, start, end) => values.copyWithin(target, start, end);",
            "copy",
            false,
            &array_copy_within,
        ),
        Some("expr:rn0,a1,a2,a3,arraycopywithin,arrayvalue".into())
    );
    let array_splice = thaw_bridge::DtsFunction {
        name: "splice".into(),
        params: vec![
            array_length.params[0].clone(),
            (
                "start".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "deleteCount".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "first".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "second".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 5,
        ret: array_length.params[0].1.clone(),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.splice = (values, start, deleteCount, first, second) => values.splice(start, deleteCount, first, second);",
            "splice",
            false,
            &array_splice,
        ),
        Some(
            "expr:rn0,a1,a2,rn0,c0000000000000000,c0000000000000000,arrayslice,a3,rnappend,a4,rnappend,arraysplice,arrayvalue"
                .into()
        )
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.splice = (values, start, deleteCount, first, second) => values.toSpliced(start, deleteCount, first, second);",
            "splice",
            false,
            &array_splice,
        ),
        Some(
            "expr:rn0,a1,a2,rn0,c0000000000000000,c0000000000000000,arrayslice,a3,rnappend,a4,rnappend,arraytospliced,arrayvalue"
                .into()
        )
    );
    let array_push = thaw_bridge::DtsFunction {
        name: "push".into(),
        params: vec![
            array_length.params[0].clone(),
            (
                "first".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "second".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 3,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.push = (values, first, second) => values.push(first, second);",
            "push",
            false,
            &array_push,
        ),
        Some("expr:rn0,a1,rnpush,drop,rn0,a2,rnpush".into())
    );
    let array_unshift = thaw_bridge::DtsFunction {
        name: "unshift".into(),
        ..array_push.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.unshift = (values, first, second) => values.unshift(first, second);",
            "unshift",
            false,
            &array_unshift,
        ),
        Some("expr:rn0,a2,rnunshift,drop,rn0,a1,rnunshift".into())
    );
    let array_pop = thaw_bridge::DtsFunction {
        name: "pop".into(),
        params: vec![array_length.params[0].clone()],
        required_params: 1,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.pop = values => values.pop();",
            "pop",
            false,
            &array_pop,
        ),
        Some("expr:rn0,rnpop".into())
    );
    let array_shift = thaw_bridge::DtsFunction {
        name: "shift".into(),
        ..array_pop.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.shift = values => values.shift();",
            "shift",
            false,
            &array_shift,
        ),
        Some("expr:rn0,rnshift".into())
    );
    let array_with = thaw_bridge::DtsFunction {
        name: "replace".into(),
        params: vec![
            array_length.params[0].clone(),
            (
                "index".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 3,
        ret: array_length.params[0].1.clone(),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.replace = (values, index, value) => values.with(index, value);",
            "replace",
            false,
            &array_with,
        ),
        Some("expr:rn0,a1,a2,rnwith,arrayvalue".into())
    );
    let array_concat = thaw_bridge::DtsFunction {
        name: "concat".into(),
        params: vec![
            array_length.params[0].clone(),
            ("other".into(), array_length.params[0].1.clone()),
        ],
        required_params: 2,
        ret: array_length.params[0].1.clone(),
        ..array_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.concat = (values, other) => values.concat(other);",
            "concat",
            false,
            &array_concat,
        ),
        Some("expr:rn0,rn1,arrayconcat,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.concat = values => values.concat();",
            "concat",
            false,
            &array_slice_all,
        ),
        Some("expr:rn0,c0000000000000000,c7ff0000000000000,arrayslice,arrayvalue".into())
    );
    let array_concat_mixed = thaw_bridge::DtsFunction {
        params: vec![
            array_length.params[0].clone(),
            (
                "first".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
            ("middle".into(), array_length.params[0].1.clone()),
            (
                "last".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 4,
        ..array_concat.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.concat = (values, first, middle, last) => values.concat(first, middle, last);",
            "concat",
            false,
            &array_concat_mixed,
        ),
        Some("expr:rn0,a1,rnappend,rn2,arrayconcat,a3,rnappend,arrayvalue".into())
    );
    for source in [
        "module.exports.length = value => parseFloat(value);",
        "module.exports.length = value => Number.parseFloat(value);",
    ] {
        assert_eq!(
            jit_numeric_export(source, "length", false, &string_length),
            Some("expr:s0,parsefloat".into())
        );
    }
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = value => parseInt(value);",
            "length",
            false,
            &string_length,
        ),
        Some("expr:s0,c0000000000000000,parseint".into())
    );
    let parse_int = thaw_bridge::DtsFunction {
        name: "parse".into(),
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "radix".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ..string_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.parse = (value, radix) => Number.parseInt(value, radix);",
            "parse",
            false,
            &parse_int,
        ),
        Some("expr:s0,a1,parseint".into())
    );
    assert_eq!(
        jit_numeric_export(
            "const parseFloat = value => 1; module.exports.length = value => parseFloat(value);",
            "length",
            false,
            &string_length,
        ),
        Some("expr:c3ff0000000000000".into())
    );
    let no_arguments = thaw_bridge::DtsFunction {
        name: "parse".into(),
        params: Vec::new(),
        required_params: 0,
        ..string_length.clone()
    };
    for (source, expected) in [
        (
            "module.exports.parse = () => parseFloat();",
            "expr:t756e646566696e6564,parsefloat",
        ),
        (
            "module.exports.parse = () => parseInt();",
            "expr:t756e646566696e6564,c0000000000000000,parseint",
        ),
    ] {
        assert_eq!(
            jit_numeric_export(source, "parse", false, &no_arguments),
            Some(expected.into())
        );
    }
    let mut shadowed_number = string_length.clone();
    shadowed_number.params[0].0 = "Number".into();
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = Number => Number('2');",
            "length",
            false,
            &shadowed_number,
        ),
        None
    );
    let string_less = thaw_bridge::DtsFunction {
        name: "less".into(),
        params: vec![
            (
                "left".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "right".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
        ],
        required_params: 2,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ..string_length.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.less = (left, right) => left < right;",
            "less",
            false,
            &string_less,
        ),
        Some("expr:s0,s1,strcmp,c0000000000000000,<".into())
    );
    let mut mixed_compare = string_less.clone();
    mixed_compare.params[1].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::F64);
    for (source, expected) in [
        (
            "module.exports.less = (left, right) => left < right;",
            "expr:s0,strnum,a1,<",
        ),
        (
            "module.exports.less = (left, right) => left == right;",
            "expr:s0,strnum,a1,==",
        ),
        (
            "module.exports.less = (left, right) => left === right;",
            "expr:s0,a1,strictfalse",
        ),
        (
            "module.exports.less = (left, right) => left !== right;",
            "expr:s0,a1,stricttrue",
        ),
        (
            "module.exports.less = function(left, right) { const normalized = left.trim(); return normalized == right; };",
            "expr:s0,trim,strnum,a1,==",
        ),
    ] {
        assert_eq!(
            jit_numeric_export(source, "less", false, &mixed_compare),
            Some(expected.into())
        );
    }
    let mut string_concat = string_length.clone();
    string_concat.name = "greet".into();
    string_concat.ret = thaw_bridge::DtsType::Native(thaw_hir::HirType::Str);
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => 'hello, ' + value + '!';",
            "greet",
            false,
            &string_concat,
        ),
        Some("expr:t68656c6c6f2c20,s0,concat,t21,concat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => 'hello, '.concat(value, '!');",
            "greet",
            false,
            &string_concat,
        ),
        Some("expr:t68656c6c6f2c20,s0,concat,t21,concat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => value.concat();",
            "greet",
            false,
            &string_concat,
        ),
        Some("expr:s0".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => `hello, ${value}!`;",
            "greet",
            false,
            &string_concat,
        ),
        Some("expr:t68656c6c6f2c20,s0,concat,t21,concat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => `value: ${1}`;",
            "greet",
            false,
            &string_concat,
        ),
        Some("expr:t76616c75653a20,c3ff0000000000000,numstr,concat".into())
    );
    let mut number_string = string_concat.clone();
    number_string.params[0].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::F64);
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => String(value);",
            "greet",
            false,
            &number_string,
        ),
        Some("expr:a0,numstr".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => String(Math.round(value / 2));",
            "greet",
            false,
            &number_string,
        ),
        Some("expr:a0,c4000000000000000,/,round,numstr".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = function(value) { const rounded = Math.round(value); return `value=${rounded}`; };",
            "greet",
            false,
            &number_string,
        ),
        Some("expr:t76616c75653d,a0,round,numstr,concat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => String(value > 0);",
            "greet",
            false,
            &number_string,
        ),
        Some("expr:a0,c0000000000000000,>,boolstr".into())
    );
    let mut shadowed_string = number_string.clone();
    shadowed_string.params[0].0 = "String".into();
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = String => String(1);",
            "greet",
            false,
            &shadowed_string,
        ),
        None
    );
    let mut boolean_string = string_concat.clone();
    boolean_string.params[0].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool);
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => `${value}`;",
            "greet",
            false,
            &boolean_string,
        ),
        Some("expr:b0,boolstr".into())
    );
    let mut mixed_concat = string_concat.clone();
    mixed_concat.params = vec![
        (
            "label".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ),
        (
            "count".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ),
        (
            "flag".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ),
    ];
    mixed_concat.required_params = 3;
    for (source, expected) in [
        (
            "module.exports.greet = (label, count, flag) => label + count + ':' + flag;",
            "expr:s0,a1,numstr,concat,t3a,concat,b2,boolstr,concat",
        ),
        (
            "module.exports.greet = (label, count, flag) => count + flag + label;",
            "expr:a1,b2,+,numstr,s0,concat",
        ),
        (
            "module.exports.greet = function(label, count, flag) { let result = label; result += count; result += flag; return result; };",
            "expr:s0,a1,numstr,concat,b2,boolstr,concat",
        ),
    ] {
        assert_eq!(
            jit_numeric_export(source, "greet", false, &mixed_concat),
            Some(expected.into())
        );
    }
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = (label, count, flag) => label.concat(count, flag);",
            "greet",
            false,
            &mixed_concat,
        ),
        Some("expr:s0,a1,numstr,concat,b2,boolstr,concat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.greet = value => '\\0' + value;",
            "greet",
            false,
            &string_concat,
        ),
        None
    );
    let mut string_predicate = string_length.clone();
    string_predicate.name = "matches".into();
    string_predicate.ret = thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool);
    for (source, expected) in [
        (
            "module.exports.matches = value => Boolean(value);",
            "expr:s0,strbool",
        ),
        (
            "module.exports.matches = value => !value;",
            "expr:s0,strbool,boolnot",
        ),
    ] {
        assert_eq!(
            jit_numeric_export(source, "matches", false, &string_predicate),
            Some(expected.into())
        );
    }
    let mut shadowed_boolean = string_predicate.clone();
    shadowed_boolean.params[0].0 = "Boolean".into();
    assert_eq!(
        jit_numeric_export(
            "module.exports.matches = Boolean => Boolean('x');",
            "matches",
            false,
            &shadowed_boolean,
        ),
        None
    );
    for (source, expected) in [
        (
            "module.exports.greet = value => value && 'yes';",
            "expr:s0,dup,strbool,&&,t796573,end",
        ),
        (
            "module.exports.greet = value => value || 'fallback';",
            "expr:s0,dup,strbool,||,t66616c6c6261636b,end",
        ),
        (
            "module.exports.greet = value => value ? 'yes' : 'no';",
            "expr:s0,strbool,if,t796573,else,t6e6f,end",
        ),
    ] {
        assert_eq!(
            jit_numeric_export(source, "greet", false, &string_concat),
            Some(expected.into())
        );
    }
    for (method, operation, search) in [
        ("startsWith", "startswith", "pre"),
        ("endsWith", "endswith", "fix"),
        ("includes", "includes", "ref"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.matches = value => value.{method}('{search}');"),
                "matches",
                false,
                &string_predicate,
            ),
            Some(format!(
                "expr:s0,t{},{operation}",
                search
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ))
        );
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.matches = value => value.{method}('{search}', 2);"),
                "matches",
                false,
                &string_predicate,
            ),
            Some(format!(
                "expr:s0,t{},c4000000000000000,{operation}2",
                search
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ))
        );
    }
    let mut coerced_search = string_predicate.clone();
    coerced_search.params.push((
        "search".into(),
        thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
    ));
    coerced_search.required_params = 2;
    assert_eq!(
        jit_numeric_export(
            "module.exports.matches = (value, search) => value.includes(search);",
            "matches",
            false,
            &coerced_search,
        ),
        Some("expr:s0,a1,numstr,includes".into())
    );
    let mut string_index = string_length.clone();
    string_index.name = "find".into();
    for (method, operation) in [("indexOf", "indexof"), ("lastIndexOf", "lastindexof")] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.find = value => value.{method}('😀');"),
                "find",
                false,
                &string_index,
            ),
            Some(format!("expr:s0,tf09f9880,{operation}"))
        );
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.find = value => value.{method}('😀', 2);"),
                "find",
                false,
                &string_index,
            ),
            Some(format!("expr:s0,tf09f9880,c4000000000000000,{operation}2"))
        );
    }
    let mut string_case = string_concat.clone();
    string_case.name = "normalize".into();
    for (method, operation) in [
        ("toLowerCase", "tolowercase"),
        ("toUpperCase", "touppercase"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.normalize = value => value.{method}();"),
                "normalize",
                false,
                &string_case,
            ),
            Some(format!("expr:s0,{operation}"))
        );
    }
    assert_eq!(
        jit_numeric_export(
            "module.exports.normalize = value => value.toWellFormed();",
            "normalize",
            false,
            &string_case,
        ),
        Some("expr:s0,towellformed".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.normalize = value => value.normalize();",
            "normalize",
            false,
            &string_case,
        ),
        Some("expr:s0,t4e4643,normalize".into())
    );
    let normalize_form = thaw_bridge::DtsFunction {
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "form".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
        ],
        required_params: 2,
        ..string_case.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.normalize = (value, form) => value.normalize(form);",
            "normalize",
            false,
            &normalize_form,
        ),
        Some("expr:s0,s1,normalize".into())
    );
    let split = thaw_bridge::DtsFunction {
        name: "split".into(),
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(Box::new(
            thaw_hir::HirType::Str,
        ))),
        ..normalize_form.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.split = (value, separator) => value.split(separator);",
            "split",
            false,
            &split,
        ),
        Some("expr:s0,s1,c7ff0000000000000,split,arrayvalue".into())
    );
    let split_limit = thaw_bridge::DtsFunction {
        params: vec![
            split.params[0].clone(),
            split.params[1].clone(),
            (
                "limit".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 3,
        ..split.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.split = (value, separator, limit) => value.split(separator, limit);",
            "split",
            false,
            &split_limit,
        ),
        Some("expr:s0,s1,a2,split,arrayvalue".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.matches = value => value.isWellFormed();",
            "matches",
            false,
            &string_predicate,
        ),
        Some("expr:s0,iswellformed".into())
    );
    let locale_compare = thaw_bridge::DtsFunction {
        name: "compare".into(),
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ..string_less.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.compare = (left, right) => left.localeCompare(right);",
            "compare",
            false,
            &locale_compare,
        ),
        Some("expr:s0,s1,strcmp".into())
    );
    for (method, operation) in [
        ("trim", "trim"),
        ("trimStart", "trimstart"),
        ("trimEnd", "trimend"),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.normalize = value => value.{method}();"),
                "normalize",
                false,
                &string_case,
            ),
            Some(format!("expr:s0,{operation}"))
        );
    }
    let string_repeat = thaw_bridge::DtsFunction {
        name: "repeat".into(),
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "count".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            ),
        ],
        required_params: 2,
        ..string_case
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.repeat = (value, count) => value.repeat(count);",
            "repeat",
            false,
            &string_repeat,
        ),
        Some("expr:s0,a1,repeat".into())
    );
    let mut string_count = string_repeat.clone();
    string_count.params[1].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::Str);
    assert_eq!(
        jit_numeric_export(
            "module.exports.repeat = (value, count) => value.repeat(count);",
            "repeat",
            false,
            &string_count,
        ),
        Some("expr:s0,s1,strnum,repeat".into())
    );
    for (method, operation) in [("slice", "slice"), ("substring", "substring")] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.repeat = (value, count) => value.{method}(count);"),
                "repeat",
                false,
                &string_repeat,
            ),
            Some(format!("expr:s0,a1,{operation}"))
        );
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.repeat = (value, count) => value.{method}();"),
                "repeat",
                false,
                &string_repeat,
            ),
            Some(format!("expr:s0,c0000000000000000,{operation}"))
        );
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.repeat = (value, count) => value.{method}(count, 4);"),
                "repeat",
                false,
                &string_repeat,
            ),
            Some(format!("expr:s0,a1,c4010000000000000,{operation}2"))
        );
    }
    let mut char_code = string_repeat.clone();
    char_code.ret = thaw_bridge::DtsType::Native(thaw_hir::HirType::F64);
    for (method, operation, signature) in [
        ("charAt", "charat", &string_repeat),
        ("charCodeAt", "charcodeat", &char_code),
    ] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.repeat = (value, count) => value.{method}(count);"),
                "repeat",
                false,
                signature,
            ),
            Some(format!("expr:s0,a1,{operation}"))
        );
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.repeat = value => value.{method}();"),
                "repeat",
                false,
                &thaw_bridge::DtsFunction {
                    params: signature.params[..1].to_vec(),
                    required_params: 1,
                    ..signature.clone()
                },
            ),
            Some(format!("expr:s0,c0000000000000000,{operation}"))
        );
    }
    for (method, operation) in [("padStart", "padstart"), ("padEnd", "padend")] {
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.repeat = (value, count) => value.{method}(count, '0');"),
                "repeat",
                false,
                &string_repeat,
            ),
            Some(format!("expr:s0,a1,t30,{operation}"))
        );
        assert_eq!(
            jit_numeric_export(
                &format!("module.exports.repeat = (value, count) => value.{method}(count);"),
                "repeat",
                false,
                &string_repeat,
            ),
            Some(format!("expr:s0,a1,t20,{operation}"))
        );
    }
    let mut coerced_pad = string_repeat.clone();
    coerced_pad.params.push((
        "pad".into(),
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
    ));
    coerced_pad.required_params = 3;
    assert_eq!(
        jit_numeric_export(
            "module.exports.repeat = (value, count, pad) => value.padEnd(count, pad);",
            "repeat",
            false,
            &coerced_pad,
        ),
        Some("expr:s0,a1,b2,boolstr,padend".into())
    );
    let string_replace = thaw_bridge::DtsFunction {
        name: "replace".into(),
        params: vec![
            (
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "search".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "replacement".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
        ],
        required_params: 3,
        ..string_repeat.clone()
    };
    for (method, operation) in [("replace", "replace"), ("replaceAll", "replaceall")] {
        assert_eq!(
            jit_numeric_export(
                &format!(
                    "module.exports.replace = (value, search, replacement) => value.{method}(search, replacement);"
                ),
                "replace",
                false,
                &string_replace,
            ),
            Some(format!("expr:s0,s1,s2,{operation}"))
        );
    }
    let mut coerced_replace = string_replace.clone();
    coerced_replace.params[1].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::F64);
    coerced_replace.params[2].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool);
    assert_eq!(
        jit_numeric_export(
            "module.exports.replace = (value, search, replacement) => value.replace(search, replacement);",
            "replace",
            false,
            &coerced_replace,
        ),
        Some("expr:s0,a1,numstr,b2,boolstr,replace".into())
    );
    let optional_string = thaw_bridge::DtsFunction {
        name: "at".into(),
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::Str,
        ))),
        ..string_repeat.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.at = (value, index) => value.at(index);",
            "at",
            false,
            &optional_string,
        ),
        Some("expr:s0,a1,at".into())
    );
    let optional_number = thaw_bridge::DtsFunction {
        name: "point".into(),
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::F64,
        ))),
        ..string_repeat.clone()
    };
    assert_eq!(
        jit_numeric_export(
            "module.exports.point = (value, index) => value.codePointAt(index);",
            "point",
            false,
            &optional_number,
        ),
        Some("expr:s0,a1,codepointat".into())
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.length = value => value + 1;",
            "length",
            false,
            &string_length,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = (Math, right) => Math.abs(Math % right);",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "console.log('side effect'); module.exports.add = (left, right) => left + right;",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "let SCALE = 2; module.exports.add = (left, right) => (left + right) * SCALE;",
            "add",
            false,
            &function,
        ),
        Some("expr:a0,a1,+,c4000000000000000,*".into())
    );
    assert_eq!(
        jit_numeric_export(
            "let SCALE = Math.random(); module.exports.add = (left, right) => (left + right) * SCALE;",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "let SCALE = LATER; let LATER = 2; module.exports.add = (left, right) => (left + right) * SCALE;",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "const SCALE = sideEffect(); module.exports.add = (left, right) => (left + right) * SCALE;",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export("module.exports = { add };", "add", false, &function,),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports = (left, right) => left - right; exports.add = (left, right) => left + right;",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = function(left, right) { const value = sideEffect(left); return value + right; };",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = function(left, right) { const value = left; value += right; return value; };",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = function(left, right) { missing = left; return missing + right; };",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports.add = function(left, right) { if (left) { console.log(right); return left; } return right; };",
            "add",
            false,
            &function,
        ),
        None
    );
    assert_eq!(
        jit_numeric_export(
            "module.exports = (left, right) => left + right;",
            "add",
            false,
            &function,
        ),
        None
    );
    let mut dispatcher = function.clone();
    dispatcher.name = "tableAlias".into();
    dispatcher.params = vec![
        (
            "name".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ),
        (
            "value".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ),
    ];
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { increment, double: twice }; function tableAlias(name, value) { return operations[name](value); } module.exports = { tableAlias };",
        "tableAlias",
        false,
        &dispatcher,
    )
    .is_some());
    let mut picked_alias = dispatcher.clone();
    picked_alias.name = "pickedAlias".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { increment, double: twice }; function pickedAlias(name, value) { const selected = operations[name]; return selected(value); } module.exports = { pickedAlias };",
        "pickedAlias",
        false,
        &picked_alias,
    )
    .is_some());
    let mut named_alias = dispatcher.clone();
    named_alias.name = "namedAlias".into();
    named_alias.params.remove(0);
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } const operations = { increment }; function namedAlias(value) { const selected = (operations.increment); return selected(value); } module.exports = { namedAlias };",
        "namedAlias",
        false,
        &named_alias,
    )
    .is_some());
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setTwice() { operations.run = twice; return 0; } function unsafeSnapshot(value) { const selected = operations.run; setTwice(); return selected(value); } module.exports = { setTwice, unsafeSnapshot };",
        "unsafeSnapshot",
        false,
        &named_alias,
    )
    .is_some());
    let mut computed_snapshot = named_alias.clone();
    computed_snapshot.name = "computedSnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setTwice() { operations.run = twice; return 0; } function computedSnapshot(value) { const selected = operations['run']; setTwice(); return selected(value); } module.exports = { setTwice, computedSnapshot };",
        "computedSnapshot",
        false,
        &computed_snapshot,
    )
    .is_some());
    let mut dynamic_snapshot = named_alias.clone();
    dynamic_snapshot.name = "dynamicSnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function dynamicSnapshot(value) { operations.run = increment; const selected = operations.run; operations.run = twice; return selected(value); } module.exports = { setDynamic, dynamicSnapshot };",
        "dynamicSnapshot",
        false,
        &dynamic_snapshot,
    )
    .is_some());
    let mut runtime_snapshot = dispatcher.clone();
    runtime_snapshot.name = "runtimeSnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function runtimeSnapshot(name, value) { setDynamic(name, false); const selected = operations[name]; setDynamic(name, true); return selected(value); } module.exports = { setDynamic, runtimeSnapshot };",
        "runtimeSnapshot",
        false,
        &runtime_snapshot,
    )
    .is_some());
    let mut reassigned_snapshot = dispatcher.clone();
    reassigned_snapshot.name = "reassignedSnapshot".into();
    reassigned_snapshot.params.insert(
        1,
        (
            "second".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
        ),
    );
    reassigned_snapshot.required_params = 3;
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function reassignedSnapshot(first, second, value) { setDynamic(first, false); setDynamic(second, true); let selected = operations[first]; selected = operations[second]; setDynamic(second, false); return selected(value); } module.exports = { setDynamic, reassignedSnapshot };",
        "reassignedSnapshot",
        false,
        &reassigned_snapshot,
    )
    .is_some());
    let mut branch_snapshot = reassigned_snapshot.clone();
    branch_snapshot.name = "branchSnapshot".into();
    branch_snapshot.params.insert(
        0,
        (
            "flag".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ),
    );
    branch_snapshot.required_params = 4;
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function fallbackIncrement(value) { return value + 1; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function branchSnapshot(flag, first, second, value) { setDynamic(first, false); setDynamic(second, true); let selected = operations[first]; if (flag) { const reached = 1; { selected = operations[second]; } } else if (first === second) { selected = operations[second]; } else { const reached = 0; { selected = fallbackIncrement; } } return setDynamic(second, false) + selected(value); } module.exports = { setDynamic, branchSnapshot };",
        "branchSnapshot",
        false,
        &branch_snapshot,
    )
    .is_some());
    let mut loop_snapshot = branch_snapshot.clone();
    loop_snapshot.name = "loopSnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function fallbackIncrement(value) { return value + 1; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function loopSnapshot(flag, first, second, value) { let selected = fallbackIncrement; let index = 0; index++; index -= 1; if (!flag) { selected = fallbackIncrement; } while (flag && index < 1) { selected = operations[second]; index++; } return setDynamic(second, false) + selected(value); } module.exports = { setDynamic, loopSnapshot };",
        "loopSnapshot",
        false,
        &loop_snapshot,
    )
    .is_some());
    let mut controlled_snapshot = branch_snapshot.clone();
    controlled_snapshot.name = "controlledSnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function controlledSnapshot(flag, first, second, value) { let selected = operations[first]; let index = 0; while (index < 2) { try { if (flag && index === 0) { selected = operations[second]; continue; } if (index === 1) break; } finally { index++; } } return setDynamic(second, false) + selected(value); } module.exports = { setDynamic, controlledSnapshot };",
        "controlledSnapshot",
        false,
        &controlled_snapshot,
    )
    .is_some());
    let mut structured_snapshot = reassigned_snapshot.clone();
    structured_snapshot.name = "structuredSnapshot".into();
    structured_snapshot.params.insert(
        0,
        (
            "mode".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
        ),
    );
    structured_snapshot.required_params = 4;
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function structuredSnapshot(mode, first, second, value) { let selected = operations[first]; let index = 0; while (index < 1) { switch (mode) { case 1: { selected = operations[second]; break; } default: { { selected = operations[first]; } } } index++; } return setDynamic(second, false) + selected(value); } module.exports = { setDynamic, structuredSnapshot };",
        "structuredSnapshot",
        false,
        &structured_snapshot,
    )
    .is_some());
    let mut catch_snapshot = branch_snapshot.clone();
    catch_snapshot.name = "catchSnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function catchSnapshot(flag, first, second, value) { let selected = operations[first]; let index = 0; while (index < 1) { try { if (flag) throw 'pick'; } catch { selected = operations[second]; } index++; } return setDynamic(second, false) + selected(value); } module.exports = { setDynamic, catchSnapshot };",
        "catchSnapshot",
        false,
        &catch_snapshot,
    )
    .is_some());
    let mut switch_snapshot = structured_snapshot.clone();
    switch_snapshot.name = "switchSnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function switchSnapshot(mode, first, second, value) { let selected = operations[first]; switch (mode) { case 1: { selected = operations[second]; break; } default: { selected = operations[first]; } } return setDynamic(second, false) + selected(value); } module.exports = { setDynamic, switchSnapshot };",
        "switchSnapshot",
        false,
        &switch_snapshot,
    )
    .is_some());
    let mut finally_snapshot = catch_snapshot.clone();
    finally_snapshot.name = "finallySnapshot".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { run: increment }; function setDynamic(name, flag) { operations[name] = flag ? twice : increment; return 0; } function finallySnapshot(flag, first, second, value) { let selected = operations[first]; try { if (flag) throw 'pick'; } catch { selected = operations[second]; } finally { setDynamic(second, false); } return selected(value); } module.exports = { setDynamic, finallySnapshot };",
        "finallySnapshot",
        false,
        &finally_snapshot,
    )
    .is_some());
    let mut branch_alias = dispatcher.clone();
    branch_alias.name = "branchAlias".into();
    branch_alias.params[0].1 = thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool);
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function branchAlias(flag, value) { let operation = twice; switch (flag) { case true: operation = twice; break; default: operation = increment; } return operation(value); } module.exports = { branchAlias };",
        "branchAlias",
        false,
        &branch_alias,
    )
    .is_some());
    let mut fixed_alias = branch_alias.clone();
    fixed_alias.name = "fixedAlias".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function fallbackIncrement(value) { return value + 1; } const fixed = { run: increment }; function setFixed(flag) { fixed.run = flag ? twice : increment; return 0; } function fixedAlias(flag, value) { let selected = fallbackIncrement; setFixed(true); if (flag) { selected = fixed.run; } return setFixed(false) + selected(value); } module.exports = { setFixed, fixedAlias };",
        "fixedAlias",
        false,
        &fixed_alias,
    )
    .is_some());
    let mut loop_alias = branch_alias.clone();
    loop_alias.name = "loopAlias".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function loopAlias(flag, value) { let operation = increment; let index = 0; while (index < 2) { if (flag && index === 1) { operation = twice; } index++; } return operation(value); } module.exports = { loopAlias };",
        "loopAlias",
        false,
        &loop_alias,
    )
    .is_some());
    for (name, source) in [
        ("forAlias", "function forAlias(flag, value) { let operation = increment; for (let index = 0; index < 2; index++) { if (flag && index === 1) operation = twice; } return operation(value); }"),
        ("doAlias", "function doAlias(flag, value) { let operation = increment; let index = 0; do { if (flag) operation = twice; index++; } while (index < 1); return operation(value); }"),
        ("forOfAlias", "function forOfAlias(flag, value) { let operation = increment; for (const item of [0, 1]) { if (flag && item === 1) operation = twice; } return operation(value); }"),
        ("forInAlias", "function forInAlias(flag, value) { let operation = increment; for (const key in { left: 1, right: 2 }) { if (flag && key === 'right') operation = twice; } return operation(value); }"),
    ] {
        let mut function = loop_alias.clone();
        function.name = name.into();
        assert!(jit_numeric_export(
            &format!("function increment(value) {{ return value + 1; }} function twice(value) {{ return value * 2; }} {source} module.exports = {{ {name} }};"),
            name,
            false,
            &function,
        )
        .is_some());
    }
    let mut controlled_alias = loop_alias.clone();
    controlled_alias.name = "controlledAlias".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function controlledAlias(flag, value) { let operation = increment; let index = 0; while (index < 2) { try { if (flag && index === 0) { operation = twice; continue; } if (index === 1) break; } finally { index++; } } return operation(value); } module.exports = { controlledAlias };",
        "controlledAlias",
        false,
        &controlled_alias,
    )
    .is_some());
    let mut conditional_alias = loop_alias.clone();
    conditional_alias.name = "conditionalLoopAlias".into();
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function conditionalLoopAlias(flag, value) { let operation = flag ? twice : increment; let index = 0; while (index < 1) { operation = flag ? increment : twice; index++; } return operation(value); } module.exports = { conditionalLoopAlias };",
        "conditionalLoopAlias",
        false,
        &conditional_alias,
    )
    .is_some());
    let mut staged_alias = branch_alias.clone();
    staged_alias.name = "stagedAlias".into();
    staged_alias.params.insert(
        1,
        (
            "reset".into(),
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool),
        ),
    );
    staged_alias.required_params = 3;
    assert!(jit_numeric_export(
        "function increment(value) { return value + 1; } function twice(value) { return value * 2; } function stagedAlias(select, reset, value) { let operation = increment; try { if (select) throw 'select'; } catch { operation = twice; } finally { if (reset) operation = increment; } return operation(value); } module.exports = { stagedAlias };",
        "stagedAlias",
        false,
        &staged_alias,
    )
    .is_some());
    assert!(
        jit_numeric_export(
            "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const operations = { increment }; function tableAlias(name, value) { operations[name] = twice; return operations[name](value); } module.exports = { tableAlias };",
            "tableAlias",
            false,
            &dispatcher,
        )
        .is_some()
    );
    assert!(
        jit_numeric_export(
            "function increment(value) { return value + 1; } function twice(value) { return value * 2; } const key = 'increment'; const operations = { increment }; operations[key] = twice; function tableAlias(name, value) { return operations[name](value); } module.exports = { tableAlias };",
            "tableAlias",
            false,
            &dispatcher,
        )
        .is_some()
    );
}
