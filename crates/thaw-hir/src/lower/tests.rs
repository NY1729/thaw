use super::*;
use crate::HirType;

fn lower(source: &str) -> HirProgram {
    let module = thaw_parser::parse_typescript(source).expect("parse error");
    lower_module(&module).expect("lowering error")
}

/// Strip the object-literal lowering scaffolding so tests can assert the literal itself:
/// per-field evaluation-order temporaries (`__thaw_object_field_N` IIFE parameters) and the
/// native-projection owner wrapper (`__thaw_full_native_owner_N`) are inlined back.
fn unstage_object_literal(expr: &HirExpr, subst: &[(String, HirExpr)]) -> HirExpr {
    let is_staging = |name: &str| {
        name.starts_with("__thaw_object_field_")
            || name.starts_with("__thaw_object_spread_")
            || name.starts_with("__thaw_full_native_owner_")
    };
    match expr {
        HirExpr::Call(callee, args) => match callee.as_ref() {
            HirExpr::Lambda(_, params, _, body)
                if params.len() == args.len() && params.iter().all(|param| is_staging(&param.name)) =>
            {
                let mut inner = subst.to_vec();
                for (param, arg) in params.iter().zip(args) {
                    inner.push((param.name.clone(), unstage_object_literal(arg, subst)));
                }
                unstage_object_literal(body, &inner)
            }
            _ => expr.clone(),
        },
        // The registration prelude only records layout; the value is the owner itself.
        HirExpr::EvalThen(_, value) if matches!(value.as_ref(), HirExpr::Var(name) if is_staging(name)) => {
            unstage_object_literal(value, subst)
        }
        HirExpr::ObjectLit(fields) => HirExpr::ObjectLit(
            fields.iter().map(|(name, value)| (name.clone(), unstage_object_literal(value, subst))).collect(),
        ),
        HirExpr::Var(name) => subst.iter().rev().find(|(bound, _)| bound == name)
            .map_or_else(|| expr.clone(), |(_, value)| value.clone()),
        other => other.clone(),
    }
}

fn unstage_let(statement: &HirStmt) -> HirStmt {
    match statement {
        HirStmt::Let(name, ty, init) => HirStmt::Let(name.clone(), ty.clone(), unstage_object_literal(init, &[])),
        other => other.clone(),
    }
}

include!("tests/async.rs");
include!("tests/classes.rs");
include!("tests/control_flow.rs");
include!("tests/core.rs");
include!("tests/ffi.rs");
include!("tests/types.rs");
include!("tests/values.rs");
