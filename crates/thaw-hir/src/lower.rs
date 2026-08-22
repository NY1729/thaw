//! SWC AST -> Thaw HIR lowering, deliberately scoped to Phase 0's "numbers,
//! strings, functions" goal: top-level function declarations with primitive
//! keyword type annotations, return/expression statements, literals,
//! identifiers, binary ops, and calls (including `console.log(...)`, which
//! has no representation of its own in HIR -- it's just encoded as a call to
//! the synthetic name "console.log" that codegen special-cases).
//!
//! There is no type inference here: every parameter and return type must be
//! an explicit primitive annotation. Real inference is thaw-typer's job
//! (tsc API), which is out of scope until Phase 1.

use swc_ecma_ast::{
    BinaryOp, CallExpr, Callee, Decl, Expr, FnDecl, Lit, MemberProp, Module, ModuleItem, Pat,
    Stmt, TsKeywordTypeKind, TsType,
};

use crate::{BinOp, HirExpr, HirFunction, HirLit, HirParam, HirProgram, HirStmt, HirType};

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let mut functions = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                functions.push(lower_fn_decl(fn_decl)?);
            }
            ModuleItem::Stmt(_) => {
                return Err(
                    "Phase 0 only supports top-level function declarations; wrap other code in a function".into(),
                )
            }
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported in Phase 0".into())
            }
        }
    }
    Ok(HirProgram { functions })
}

fn lower_fn_decl(fn_decl: &FnDecl) -> Result<HirFunction, String> {
    let name = fn_decl.ident.sym.to_string();
    let func = &fn_decl.function;

    let params = func
        .params
        .iter()
        .map(|param| lower_param(&param.pat))
        .collect::<Result<Vec<_>, _>>()?;

    let ret = match &func.return_type {
        Some(ann) => lower_ts_type(&ann.type_ann)?,
        None => HirType::Void,
    };

    let body_block = func
        .body
        .as_ref()
        .ok_or_else(|| format!("function `{name}` has no body (ambient/overload decl?)"))?;

    let body = body_block
        .stmts
        .iter()
        .map(lower_stmt)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(HirFunction {
        name,
        params,
        ret,
        body,
    })
}

fn lower_param(pat: &Pat) -> Result<HirParam, String> {
    let Pat::Ident(binding) = pat else {
        return Err("only simple identifier parameters are supported in Phase 0".into());
    };
    let name = binding.id.sym.to_string();
    let ty = match &binding.type_ann {
        Some(ann) => lower_ts_type(&ann.type_ann)?,
        None => {
            return Err(format!(
                "parameter `{name}` needs an explicit type annotation (no type inference yet)"
            ))
        }
    };
    Ok(HirParam { name, ty })
}

fn lower_ts_type(ty: &TsType) -> Result<HirType, String> {
    match ty {
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => Ok(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => Ok(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => Ok(HirType::Bool),
            TsKeywordTypeKind::TsVoidKeyword => Ok(HirType::Void),
            other => Err(format!(
                "unsupported type keyword {other:?} (Phase 0 supports number/string/boolean/void)"
            )),
        },
        other => Err(format!(
            "unsupported type annotation {other:?} (Phase 0 supports only primitive keyword types)"
        )),
    }
}

fn lower_stmt(stmt: &Stmt) -> Result<HirStmt, String> {
    match stmt {
        Stmt::Return(ret) => {
            let value = ret.arg.as_deref().map(lower_expr).transpose()?;
            Ok(HirStmt::Return(value))
        }
        Stmt::Expr(expr_stmt) => Ok(HirStmt::Expr(lower_expr(&expr_stmt.expr)?)),
        other => Err(format!(
            "unsupported statement {other:?} (Phase 0 supports return/expression statements only)"
        )),
    }
}

fn lower_expr(expr: &Expr) -> Result<HirExpr, String> {
    match expr {
        Expr::Lit(Lit::Num(n)) => Ok(HirExpr::Lit(HirLit::F64(n.value))),
        Expr::Lit(Lit::Str(s)) => Ok(HirExpr::Lit(HirLit::Str(
            s.value.to_string_lossy().into_owned(),
        ))),
        Expr::Lit(Lit::Bool(b)) => Ok(HirExpr::Lit(HirLit::Bool(b.value))),
        Expr::Ident(ident) => Ok(HirExpr::Var(ident.sym.to_string())),
        Expr::Paren(paren) => lower_expr(&paren.expr),
        Expr::Bin(bin) => {
            let op = lower_bin_op(bin.op)?;
            let lhs = lower_expr(&bin.left)?;
            let rhs = lower_expr(&bin.right)?;
            Ok(HirExpr::BinOp(op, Box::new(lhs), Box::new(rhs)))
        }
        Expr::Call(call) => lower_call(call),
        other => Err(format!(
            "unsupported expression {other:?} (Phase 0 supports literals, identifiers, binary ops, and calls)"
        )),
    }
}

fn lower_bin_op(op: BinaryOp) -> Result<BinOp, String> {
    match op {
        BinaryOp::Add => Ok(BinOp::Add),
        BinaryOp::Sub => Ok(BinOp::Sub),
        BinaryOp::Mul => Ok(BinOp::Mul),
        BinaryOp::Div => Ok(BinOp::Div),
        BinaryOp::Lt => Ok(BinOp::Lt),
        BinaryOp::Gt => Ok(BinOp::Gt),
        BinaryOp::EqEqEq => Ok(BinOp::EqEqEq),
        other => Err(format!("unsupported binary operator {other:?}")),
    }
}

fn lower_call(call: &CallExpr) -> Result<HirExpr, String> {
    let Callee::Expr(callee_expr) = &call.callee else {
        return Err("unsupported callee (super/import calls not supported)".into());
    };

    let callee_name = match callee_expr.as_ref() {
        Expr::Ident(ident) => ident.sym.to_string(),
        // `console.log` has no dedicated HIR node; it's encoded as a call to
        // the synthetic name "console.log" and codegen special-cases it.
        Expr::Member(member) => {
            let Expr::Ident(obj) = member.obj.as_ref() else {
                return Err("unsupported member call target".into());
            };
            let MemberProp::Ident(prop) = &member.prop else {
                return Err("unsupported member call property".into());
            };
            format!("{}.{}", obj.sym, prop.sym)
        }
        _ => {
            return Err(
                "unsupported call target (only plain identifiers and console.log are supported)"
                    .into(),
            )
        }
    };

    let args = call
        .args
        .iter()
        .map(|arg| {
            if arg.spread.is_some() {
                return Err("spread arguments are not supported in Phase 0".to_string());
            }
            lower_expr(&arg.expr)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(HirExpr::Call(Box::new(HirExpr::Var(callee_name)), args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower(source: &str) -> HirProgram {
        let module = thaw_parser::parse_typescript(source).expect("parse error");
        lower_module(&module).expect("lowering error")
    }

    #[test]
    fn lowers_typed_function_with_binary_op() {
        let program = lower("function add(a: number, b: number): number { return a + b; }");
        assert_eq!(program.functions.len(), 1);
        let f = &program.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(
            f.params,
            vec![
                HirParam {
                    name: "a".into(),
                    ty: HirType::F64
                },
                HirParam {
                    name: "b".into(),
                    ty: HirType::F64
                },
            ]
        );
        assert_eq!(f.ret, HirType::F64);
        assert_eq!(
            f.body,
            vec![HirStmt::Return(Some(HirExpr::BinOp(
                BinOp::Add,
                Box::new(HirExpr::Var("a".into())),
                Box::new(HirExpr::Var("b".into())),
            )))]
        );
    }

    #[test]
    fn lowers_console_log_of_a_string_literal() {
        let program =
            lower(r#"function main(): void { console.log("Hello, Thaw!"); }"#);
        let f = &program.functions[0];
        assert_eq!(
            f.body,
            vec![HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::Lit(HirLit::Str("Hello, Thaw!".into()))],
            ))]
        );
    }

    #[test]
    fn rejects_missing_parameter_type_annotation() {
        let module = thaw_parser::parse_typescript("function f(a) { return a; }").unwrap();
        assert!(lower_module(&module).is_err());
    }
}
