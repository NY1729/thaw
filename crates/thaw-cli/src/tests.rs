use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

include!("tests/static_build.rs");

include!("tests/module_graph.rs");

include!("tests/registry_modules.rs");

include!("tests/ffi_metadata.rs");

include!("tests/native_addons.rs");

#[test]
fn adapts_typed_dynamic_callable_results_to_natural_calls() {
    let function = thaw_bridge::DtsFunction {
        name: "customAlphabet".into(),
        generic: None,
        params: vec![
            (
                "alphabet".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
            ),
            (
                "defaultSize".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
                    thaw_hir::HirType::F64,
                ))),
            ),
        ],
        required_params: 1,
        rest_param: None,
        ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::CallableFunction(
            vec![thaw_hir::HirType::Optional(Box::new(
                thaw_hir::HirType::F64,
            ))],
            thaw_hir::HirOptionalMask::from_bools(&[true]),
            None,
            Box::new(thaw_hir::HirType::Str),
        )),
    };
    let (target, shim) = typed_dynamic_declaration("nanoid", &function, false).unwrap();
    assert!(target.starts_with("__thaw_typed_callable_"));
    assert!(shim.contains("defaultSize?: number | undefined"));
    assert!(shim.contains("const invoke: (arg0?: number | undefined) => string"));
    assert!(shim.contains("callDynamicValue(callable"));

    let (_, napi_shim) = typed_dynamic_declaration("native", &function, true).unwrap();
    assert!(napi_shim.contains("callNativeAddonValue(callable"));
}

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
            "function add(left, right) { return add(left, right); } module.exports.add = add;",
            "add",
            false,
            &function,
        ),
        None
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
        Some("expr:b0,c0000000000000000,c3ff0000000000000,?,asbool".into())
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
            "expr:s0,strbool,c0000000000000000,c3ff0000000000000,?,asbool",
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
}

#[test]
fn rewrite_qualified_calls_is_a_no_op_with_no_rewrites() {
    let source = "function main(): void { console.log(qs.stringify(x)); }";
    assert_eq!(rewrite_qualified_calls(source, &[]).unwrap(), source);
}

#[test]
fn rewrite_qualified_calls_replaces_matching_qualified_calls_only() {
    let source = "function main(): void {\n\
             console.log(qs.stringify(x));\n\
             console.log(hoek.stringify(y));\n\
             console.log(qs.parse(z));\n\
             console.log(unrelated.stringify(w));\n\
         }";
    let rewrites = vec![
        (
            "qs".to_string(),
            "stringify".to_string(),
            "qs_stringify".to_string(),
        ),
        (
            "hoek".to_string(),
            "stringify".to_string(),
            "hoek_stringify".to_string(),
        ),
    ];
    let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();

    assert!(rewritten.contains("console.log(qs_stringify(x));"));
    assert!(rewritten.contains("console.log(hoek_stringify(y));"));
    // Not in `rewrites` (no collision for `parse`, or the object
    // isn't a known qualifier at all) -- left completely alone.
    assert!(rewritten.contains("console.log(qs.parse(z));"));
    assert!(rewritten.contains("console.log(unrelated.stringify(w));"));
}

#[test]
fn rewrite_qualified_calls_handles_a_call_nested_in_an_expression() {
    let source = "function main(): void { const r = String(qs.stringify(x)); }";
    let rewrites = vec![(
        "qs".to_string(),
        "stringify".to_string(),
        "qs_stringify".to_string(),
    )];
    let rewritten = rewrite_qualified_calls(source, &rewrites).unwrap();
    assert!(rewritten.contains("const r = String(qs_stringify(x));"));
}

#[test]
fn rewrites_external_class_constructors_without_touching_other_new_expressions() {
    let source = "const a = new Database(\":memory:\"); const b = new sqlite3.Database(\"db.sqlite\"); const c = new LocalBox(1);";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![(1, "Database_ctor".into(), vec![])],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const a = Database_ctor(\":memory:\"); const b = Database_ctor(\"db.sqlite\"); const c = new LocalBox(1);"
        );
}

#[test]
fn rewrites_external_class_constructors_by_argument_count() {
    let source = "const a = new Database(); const b = new Database('db'); const c = new Database('db', 6); const d = new Database('db', 6, true);";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![
                (0, "Database_ctor0".into(), vec![]),
                (1, "Database_ctor1".into(), vec![]),
                (2, "Database_ctor2".into(), vec![]),
            ],
        )],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const a = Database_ctor0(); const b = Database_ctor1('db'); const c = Database_ctor2('db', 6); const d = new Database('db', 6, true);"
    );
}

#[test]
fn does_not_guess_between_same_arity_constructor_helpers() {
    let source = "const value = new NativeBox(true);";
    let rewritten = rewrite_external_class_constructors(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![
                (1, "NativeBox_string".into(), vec![thaw_hir::HirType::Str]),
                (1, "NativeBox_number".into(), vec![thaw_hir::HirType::F64]),
            ],
        )],
    )
    .unwrap();
    assert_eq!(rewritten, source);
}

#[test]
fn generates_napi_constructor_helpers_for_each_supported_arity() {
    let class = thaw_bridge::DtsClass {
        name: "Client".into(),
        extends: None,
        constructible: true,
        constructors: vec![thaw_bridge::DtsConstructor {
            params: vec![
                (
                    "url".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
                ),
                (
                    "timeout".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                ),
            ],
            required_params: 0,
            overloaded: false,
        }],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, &mut shim);
    assert_eq!(
        helpers
            .iter()
            .map(|(arity, _, _)| *arity)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(shim.contains("(): JsValue;"));
    assert!(shim.contains("(url: string): JsValue;"));
    assert!(shim.contains("(url: string, timeout: number): JsValue;"));

    let rewritten = rewrite_external_class_constructors(
        "const a = new Client(); const b = new Client('x'); const c = new Client('x', 5);",
        &[("pkg".into(), "Client".into(), helpers.clone())],
    )
    .unwrap();
    for (_, helper, _) in helpers {
        assert!(rewritten.contains(&helper));
    }

    let mut default_shim = String::new();
    let default_helpers = generate_napi_class_constructors(
        &thaw_bridge::DtsClass {
            name: "DefaultBox".into(),
            extends: None,
            constructible: true,
            constructors: vec![],
            methods: vec![],
            properties: vec![],
        },
        &mut default_shim,
    );
    assert_eq!(default_helpers.len(), 1);
    assert_eq!(default_helpers[0].0, 0);
    assert!(default_shim.contains("(): JsValue;"));

    let mut locked_shim = String::new();
    let mut locked = class;
    locked.constructible = false;
    assert!(generate_napi_class_constructors(&locked, &mut locked_shim).is_empty());
    assert!(locked_shim.is_empty());
}

#[test]
fn selects_same_arity_napi_constructors_by_argument_type() {
    let class = thaw_bridge::DtsClass {
        name: "NativeBox".into(),
        extends: None,
        constructible: true,
        constructors: vec![
            thaw_bridge::DtsConstructor {
                params: vec![(
                    "value".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Str),
                )],
                required_params: 1,
                overloaded: true,
            },
            thaw_bridge::DtsConstructor {
                params: vec![(
                    "value".into(),
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
                )],
                required_params: 1,
                overloaded: true,
            },
        ],
        methods: vec![],
        properties: vec![],
    };
    let mut shim = String::new();
    let helpers = generate_napi_class_constructors(&class, &mut shim);
    assert_eq!(helpers.len(), 2);
    let string_helper = helpers
        .iter()
        .find(|(_, _, types)| types == &[thaw_hir::HirType::Str])
        .unwrap()
        .1
        .clone();
    let number_helper = helpers
        .iter()
        .find(|(_, _, types)| types == &[thaw_hir::HirType::F64])
        .unwrap()
        .1
        .clone();
    let rewritten = rewrite_external_class_methods(
        "const n = 42; const s = 'value'; const a = new NativeBox(n); const b = new NativeBox(s); const c = new NativeBox(7); const d = new NativeBox('text');",
        &[("pkg".into(), "NativeBox".into(), helpers)],
        &[],
    )
    .unwrap();
    assert!(rewritten.contains(&format!("const a = {number_helper}(n)")));
    assert!(rewritten.contains(&format!("const b = {string_helper}(s)")));
    assert!(rewritten.contains(&format!("const c = {number_helper}(7)")));
    assert!(rewritten.contains(&format!("const d = {string_helper}('text')")));
}

#[test]
fn generates_typed_napi_tuple_class_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class PairBox {
                constructor(value: [number, string]);
                swap(value: [number, string]): [string, number];
                pair: [number, string];
            }"#,
    )
    .unwrap()
    .remove(0);
    let tuple = thaw_hir::HirType::Tuple(vec![thaw_hir::HirType::F64, thaw_hir::HirType::Str]);
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, &mut shim);
    assert_eq!(constructors[0].2, vec![tuple.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].4, vec![tuple.clone()]);
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: [number, string]"));
    assert!(shim.contains("): [string, number]"));

    let rewritten = rewrite_external_class_methods(
        "const value = unknown as [number, string]; const box = new PairBox(value); box.swap(value);",
        &[("pkg".into(), "PairBox".into(), constructors)],
        &[(
            "PairBox".into(),
            "swap".into(),
            methods[0].1.clone(),
            1,
            false,
            vec![tuple],
        )],
    )
    .unwrap();
    assert!(!rewritten.contains("new PairBox"));
    assert!(rewritten.contains(&methods[0].1));
}

#[test]
fn generates_typed_napi_recursive_array_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class ArrayBox {
                constructor(labels: string[]);
                flags(values: boolean[]): string[];
                group(values: string[][]): string[][];
                records: { name: string }[];
            }"#,
    )
    .unwrap()
    .remove(0);
    let strings = thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Str));
    let booleans = thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Bool));
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, &mut shim);
    assert_eq!(constructors[0].2, vec![strings.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    assert_eq!(methods[0].4, vec![booleans]);
    assert_eq!(
        methods[1].4,
        vec![thaw_hir::HirType::Array(Box::new(strings.clone()))]
    );
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: { name: string }[]"));
    assert!(shim.contains("values: boolean[]"));
    assert!(shim.contains("values: string[][]"));

    let rewritten = rewrite_external_class_methods(
        "const box = new ArrayBox([\"a\"]); box.flags([true, false]); box.group([[\"a\"]]);",
        &[("pkg".into(), "ArrayBox".into(), constructors)],
        &[
            (
                "ArrayBox".into(),
                "flags".into(),
                methods[0].1.clone(),
                1,
                false,
                methods[0].4.clone(),
            ),
            (
                "ArrayBox".into(),
                "group".into(),
                methods[1].1.clone(),
                1,
                false,
                methods[1].4.clone(),
            ),
        ],
    )
    .unwrap();
    assert!(!rewritten.contains("new ArrayBox"));
    assert!(rewritten.contains(&methods[0].1));
    assert!(rewritten.contains(&methods[1].1));
}

#[test]
fn generates_typed_napi_nullable_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class NullableBox {
                constructor(value: string | null);
                normalize(value: string | null): string | null;
                value: string | null;
            }"#,
    )
    .unwrap()
    .remove(0);
    let nullable = thaw_hir::HirType::Nullable(Box::new(thaw_hir::HirType::Str));
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, &mut shim);
    assert_eq!(constructors[0].2, vec![nullable.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    assert_eq!(methods[0].4, vec![nullable]);
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: string | null"));
    assert!(shim.contains("): string | null"));

    let rewritten = rewrite_external_class_methods(
        "const box = new NullableBox(null); box.normalize(null);",
        &[("pkg".into(), "NullableBox".into(), constructors)],
        &[(
            "NullableBox".into(),
            "normalize".into(),
            methods[0].1.clone(),
            1,
            false,
            methods[0].4.clone(),
        )],
    )
    .unwrap();
    assert!(!rewritten.contains("new NullableBox"));
    assert!(rewritten.contains(&methods[0].1));
}

#[test]
fn generates_typed_napi_optional_and_nullish_shims() {
    let class = thaw_bridge::parse_dts_classes(
        r#"export class OptionalBox {
                constructor(value: string | undefined);
                normalize(value: string | undefined): string | undefined;
                mixed(value: string | null | undefined): string | null | undefined;
                value: string | undefined;
            }"#,
    )
    .unwrap()
    .remove(0);
    let optional = thaw_hir::HirType::Optional(Box::new(thaw_hir::HirType::Str));
    let nullish = thaw_hir::HirType::Nullish(Box::new(thaw_hir::HirType::Str));
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(&class, &mut shim);
    assert_eq!(constructors[0].2, vec![optional.clone()]);
    let methods = generate_napi_class_method_overloads(
        &class,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    assert_eq!(methods[0].4, vec![optional]);
    assert_eq!(methods[1].4, vec![nullish]);
    let property = &class.properties[0];
    assert!(generate_napi_class_property_getter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(generate_napi_class_property_setter(
        &class.name,
        &property.name,
        &property.ty,
        false,
        &mut shim,
    )
    .is_some());
    assert!(shim.contains("value: string | undefined"));
    assert!(shim.contains("value: string | null | undefined"));

    let rewritten = rewrite_external_class_methods(
        "const box = new OptionalBox(undefined); box.normalize(undefined); box.mixed(null);",
        &[("pkg".into(), "OptionalBox".into(), constructors)],
        &[
            (
                "OptionalBox".into(),
                "normalize".into(),
                methods[0].1.clone(),
                1,
                false,
                methods[0].4.clone(),
            ),
            (
                "OptionalBox".into(),
                "mixed".into(),
                methods[1].1.clone(),
                1,
                false,
                methods[1].4.clone(),
            ),
        ],
    )
    .unwrap();
    assert!(!rewritten.contains("new OptionalBox"));
    assert!(rewritten.contains(&methods[0].1));
    assert!(rewritten.contains(&methods[1].1));
}

#[test]
fn generates_napi_class_property_accessor_helpers() {
    let mut shim = String::new();
    let ty = thaw_bridge::DtsType::Native(thaw_hir::HirType::Str);
    let instance_getter =
        generate_napi_class_property_getter("Client", "name", &ty, false, &mut shim).unwrap();
    let (instance_setter, setter_type) =
        generate_napi_class_property_setter("Client", "name", &ty, false, &mut shim).unwrap();
    let static_getter =
        generate_napi_class_property_getter("Client", "version", &ty, true, &mut shim).unwrap();
    let (static_setter, _) =
        generate_napi_class_property_setter("Client", "version", &ty, true, &mut shim).unwrap();

    assert_eq!(setter_type, thaw_hir::HirType::Str);
    assert!(shim.contains(&format!(
        "declare function {instance_getter}(receiver: JsValue): string;"
    )));
    assert!(shim.contains(&format!(
        "declare function {instance_setter}(receiver: JsValue, value: string): string;"
    )));
    assert!(shim.contains(&format!("declare function {static_getter}(): string;")));
    assert!(shim.contains(&format!(
        "declare function {static_setter}(value: string): string;"
    )));
    assert!(generate_napi_class_property_getter(
        "Client",
        "optional",
        &thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(Box::new(
            thaw_hir::HirType::Str,
        ))),
        false,
        &mut shim,
    )
    .is_some());
}

#[test]
fn rewrites_inherited_external_class_methods() {
    let classes = thaw_bridge::parse_dts_classes(
        r#"export class Base {
                constructor(value: number);
                inherited(value: number): number;
            }
            export class Derived extends Base {}"#,
    )
    .unwrap();
    let derived = classes
        .iter()
        .find(|class| class.name == "Derived")
        .unwrap();
    let mut shim = String::new();
    let constructors = generate_napi_class_constructors(derived, &mut shim);
    assert_eq!(constructors.len(), 1);
    assert_eq!(constructors[0].0, 1);
    let generated = generate_napi_class_method_overloads(
        derived,
        false,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    let inherited = generated
        .iter()
        .find(|(method, _, _, _, _)| method == "inherited")
        .unwrap();
    let rewritten = rewrite_external_class_methods(
        "const value = new Derived(4); value.inherited(4);",
        &[(
            "pkg".into(),
            "Derived".into(),
            vec![(1, "Derived_ctor".into(), vec![])],
        )],
        &[(
            "Derived".into(),
            inherited.0.clone(),
            inherited.1.clone(),
            inherited.2,
            inherited.3,
            inherited.4.clone(),
        )],
    )
    .unwrap();
    assert!(rewritten.contains(&format!("{}(value, 4)", inherited.1)));
}

#[test]
fn rewrites_methods_on_values_created_from_external_classes() {
    let source = "const db = new Database(\":memory:\"); db.configure(\"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![(1, "Database_ctor".into(), vec![])],
        )],
        &[(
            "Database".into(),
            "configure".into(),
            "__thaw_configure".into(),
            2,
            false,
            vec![thaw_hir::HirType::Str, thaw_hir::HirType::F64],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const db = Database_ctor(\":memory:\"); __thaw_configure(db, \"busyTimeout\", 1000); const local = new LocalBox(1); local.configure(2);"
        );
}

#[test]
fn rewrites_named_and_namespace_static_class_methods() {
    let source = "NativeBox.create(1); addon.NativeBox.create(\"text\"); LocalBox.create(2);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[],
        &[],
        &[
            (
                "addon".into(),
                "NativeBox".into(),
                "create".into(),
                "__thaw_create_number".into(),
                1,
                false,
                vec![thaw_hir::HirType::F64],
            ),
            (
                "addon".into(),
                "NativeBox".into(),
                "create".into(),
                "__thaw_create_string".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
        ],
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "__thaw_create_number(1); __thaw_create_string(\"text\"); LocalBox.create(2);"
    );
}

#[test]
fn rewrites_typed_napi_instance_getters() {
    let source = "const box = new NativeBox(42); console.log(box.value);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[],
        &[],
        &[(
            "NativeBox".into(),
            "value".into(),
            "__thaw_get_value".into(),
        )],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = NativeBox_ctor(42); console.log(__thaw_get_value(box));"
    );
}

#[test]
fn rewrites_typed_napi_instance_setters_and_preserves_expression_values() {
    let source = "const box = new NativeBox(42); const assigned: number = box.value = 7;";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[],
        &[],
        &[],
        &[(
            "NativeBox".into(),
            "value".into(),
            "__thaw_set_value".into(),
            thaw_hir::HirType::F64,
        )],
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = NativeBox_ctor(42); const assigned: number = __thaw_set_value(box, 7);"
    );
}

#[test]
fn rewrites_named_and_namespace_static_accessors() {
    let source = "console.log(NativeBox.version); addon.NativeBox.version = 7; console.log(addon.NativeBox.version);";
    let rewritten = rewrite_external_class_methods_with_static(
        source,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[(
            "addon".into(),
            "NativeBox".into(),
            "version".into(),
            "__thaw_get_version".into(),
        )],
        &[(
            "addon".into(),
            "NativeBox".into(),
            "version".into(),
            "__thaw_set_version".into(),
            thaw_hir::HirType::F64,
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "console.log(__thaw_get_version()); __thaw_set_version(7); console.log(__thaw_get_version());"
        );
}

#[test]
fn generates_typed_napi_static_method_shims_without_instance_receivers() {
    let class = thaw_bridge::DtsClass {
        name: "NativeBox".into(),
        extends: None,
        constructible: true,
        constructors: vec![],
        methods: vec![thaw_bridge::DtsMethod {
            name: "create".into(),
            params: vec![(
                "value".into(),
                thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            )],
            required_params: 1,
            rest_param: None,
            ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::F64),
            is_static: true,
            kind: thaw_bridge::DtsMethodKind::Method,
            overloaded: false,
        }],
        properties: vec![],
    };
    let mut shim = String::new();
    let generated = generate_napi_class_method_overloads(
        &class,
        true,
        &std::collections::HashMap::new(),
        &mut shim,
    );
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].0, "create");
    assert!(shim.contains("(value: number): number;"));
    assert!(!shim.contains("receiver"));
}

#[test]
fn rewrites_zero_argument_external_class_methods() {
    let source = "const box = new NativeBox(42); const value = box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const box = NativeBox_ctor(42); const value = __thaw_get(box);"
    );
}

#[test]
fn tracks_external_class_instance_aliases_and_invalidates_reassignments() {
    let source = "const box = new NativeBox(42); const alias = box; alias.get(); let assigned = alias; assigned.get(); assigned = box; assigned.get(); assigned = unknown; assigned.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = NativeBox_ctor(42); const alias = box; __thaw_get(alias); let assigned = alias; __thaw_get(assigned); assigned = box; __thaw_get(assigned); assigned = unknown; assigned.get();"
        );
}

#[test]
fn tracks_external_class_instances_through_object_properties() {
    let source = "const box = new NativeBox(42); const holder = { box }; holder.box.get(); holder[\"box\"].get(); const nested = { inner: { value: new NativeBox(7) } }; nested.inner.value.get(); holder.box = box; holder.box.get(); holder.box = unknown; holder.box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = NativeBox_ctor(42); const holder = { box }; __thaw_get(holder.box); __thaw_get(holder[\"box\"]); const nested = { inner: { value: NativeBox_ctor(7) } }; __thaw_get(nested.inner.value); holder.box = box; __thaw_get(holder.box); holder.box = unknown; holder.box.get();"
        );
}

#[test]
fn joins_object_property_instance_facts_across_branches() {
    let source = "const box = new NativeBox(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } holder.box.get(); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "addon".into(),
            "NativeBox".into(),
            vec![(1, "NativeBox_ctor".into(), vec![])],
        )],
        &[(
            "NativeBox".into(),
            "get".into(),
            "__thaw_get".into(),
            0,
            false,
            vec![],
        )],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const box = NativeBox_ctor(42); let holder = { box }; if (flag) { holder.box = box; } else { holder.box = box; } __thaw_get(holder.box); if (flag) { holder.box = box; } else { holder.box = unknown; } holder.box.get(); holder = unknown; holder.box.get();"
        );
}

#[test]
fn selects_external_method_overloads_by_arity_and_callback_shape() {
    let source = "const db = new Database(\":memory:\"); const done = (error: Json): void => {}; db.run(\"select 1\"); db.run(\"select 1\", done);";
    let rewritten = rewrite_external_class_methods(
        source,
        &[(
            "sqlite3".into(),
            "Database".into(),
            vec![(1, "Database_ctor".into(), vec![])],
        )],
        &[
            (
                "Database".into(),
                "run".into(),
                "__run_sync".into(),
                1,
                false,
                vec![thaw_hir::HirType::Str],
            ),
            (
                "Database".into(),
                "run".into(),
                "__run_callback".into(),
                2,
                true,
                vec![
                    thaw_hir::HirType::Str,
                    thaw_hir::HirType::Function(
                        vec![thaw_hir::HirType::Json],
                        Box::new(thaw_hir::HirType::Void),
                    ),
                ],
            ),
        ],
    )
    .unwrap();
    assert_eq!(
            rewritten,
            "const db = Database_ctor(\":memory:\"); const done = (error: Json): void => {}; __run_sync(db, \"select 1\"); __run_callback(db, \"select 1\", done);"
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
