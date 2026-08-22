//! SWC AST -> Thaw HIR lowering.
//!
//! Phase 0 scope (still true here): no type inference for function
//! params/returns -- those need explicit primitive annotations. Phase 1
//! adds: `if`, `while`, classic C-style `for` (desugared to `let` +
//! `while`), `throw`/`try`/`catch` (same-function-scope only, see
//! `HirStmt::Try`), local `let`/`const` (annotation-or-literal-inferred),
//! assignment/compound-assignment/`++`/`--`, and number arrays (`number[]`).
//!
//! A single SWC `Stmt` can lower to *several* HIR statements (`for` becomes
//! a `Let` followed by a `While`), so the statement lowering entry point is
//! [`lower_stmt_seq`], not a single-statement `lower_stmt`.

use swc_ecma_ast::{
    AssignOp, AssignTarget, BinaryOp, CallExpr, Callee, Decl, Expr, FnDecl, Lit, MemberProp,
    Module, ModuleItem, Pat, SimpleAssignTarget, Stmt, TsKeywordTypeKind, TsType, UpdateOp,
    VarDeclOrExpr,
};

use crate::{BinOp, HirExpr, HirFunction, HirLit, HirParam, HirProgram, HirStmt, HirType, Symbol};

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let mut functions = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                functions.push(lower_fn_decl(fn_decl)?);
            }
            ModuleItem::Stmt(_) => {
                return Err(
                    "Phase 0/1 only support top-level function declarations; wrap other code in a function"
                        .into(),
                )
            }
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
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

    let body = lower_stmts(&body_block.stmts)?;

    Ok(HirFunction {
        name,
        params,
        ret,
        body,
    })
}

fn lower_param(pat: &Pat) -> Result<HirParam, String> {
    let Pat::Ident(binding) = pat else {
        return Err("only simple identifier parameters are supported".into());
    };
    let name = binding.id.sym.to_string();
    let ty = match &binding.type_ann {
        Some(ann) => lower_ts_type(&ann.type_ann)?,
        None => {
            return Err(format!(
                "parameter `{name}` needs an explicit type annotation (no type inference for params)"
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
                "unsupported type keyword {other:?} (supports number/string/boolean/void)"
            )),
        },
        TsType::TsArrayType(arr) => Ok(HirType::Array(Box::new(lower_ts_type(&arr.elem_type)?))),
        TsType::TsTypeRef(ty_ref) => {
            // Accept `Array<T>` as an alternate spelling of `T[]`. This is
            // not general generics support (still deferred per the
            // roadmap) -- just recognizing the one built-in spelling.
            let is_array_ref = matches!(&ty_ref.type_name, swc_ecma_ast::TsEntityName::Ident(id) if id.sym == *"Array");
            if is_array_ref {
                if let Some(params) = &ty_ref.type_params {
                    if let [elem] = params.params.as_slice() {
                        return Ok(HirType::Array(Box::new(lower_ts_type(elem)?)));
                    }
                }
            }
            Err("unsupported type reference (generics are not supported yet)".into())
        }
        other => Err(format!(
            "unsupported type annotation {other:?} (supports primitive keywords and T[]/Array<T>)"
        )),
    }
}

/// Lowers a block's statement list, flattening statements that expand to
/// more than one HIR statement (namely `for`).
fn lower_stmts(stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
    let mut out = Vec::new();
    for stmt in stmts {
        out.extend(lower_stmt_seq(stmt)?);
    }
    Ok(out)
}

/// Normalizes a `for`/`while`/`if` body, which SWC represents as a single
/// `Stmt` (either a `{ ... }` block or one bare statement), into a flat HIR
/// statement list.
fn lower_body(stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
    match stmt {
        Stmt::Block(block) => lower_stmts(&block.stmts),
        other => lower_stmt_seq(other),
    }
}

fn lower_stmt_seq(stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
    match stmt {
        Stmt::Return(ret) => {
            let value = ret.arg.as_deref().map(lower_expr).transpose()?;
            Ok(vec![HirStmt::Return(value)])
        }
        Stmt::Expr(expr_stmt) => Ok(vec![HirStmt::Expr(lower_expr(&expr_stmt.expr)?)]),
        Stmt::Block(block) => lower_stmts(&block.stmts),
        Stmt::Decl(Decl::Var(var_decl)) => lower_var_decl(var_decl),

        Stmt::If(if_stmt) => {
            let cond = lower_expr(&if_stmt.test)?;
            let then_branch = lower_body(&if_stmt.cons)?;
            let else_branch = match &if_stmt.alt {
                Some(alt) => lower_body(alt)?,
                None => Vec::new(),
            };
            Ok(vec![HirStmt::If(cond, then_branch, else_branch)])
        }

        Stmt::While(while_stmt) => {
            let cond = lower_expr(&while_stmt.test)?;
            let body = lower_body(&while_stmt.body)?;
            Ok(vec![HirStmt::While(cond, body)])
        }

        Stmt::For(for_stmt) => {
            let mut out = Vec::new();
            if let Some(init) = &for_stmt.init {
                match init {
                    VarDeclOrExpr::VarDecl(var_decl) => out.extend(lower_var_decl(var_decl)?),
                    VarDeclOrExpr::Expr(expr) => out.push(HirStmt::Expr(lower_expr(expr)?)),
                }
            }

            let cond = match &for_stmt.test {
                Some(test) => lower_expr(test)?,
                None => HirExpr::Lit(HirLit::Bool(true)),
            };

            let mut body = lower_body(&for_stmt.body)?;
            if let Some(update) = &for_stmt.update {
                body.push(HirStmt::Expr(lower_expr(update)?));
            }

            out.push(HirStmt::While(cond, body));
            Ok(out)
        }

        Stmt::Throw(throw_stmt) => Ok(vec![HirStmt::Throw(lower_expr(&throw_stmt.arg)?)]),

        Stmt::Try(try_stmt) => {
            if try_stmt.finalizer.is_some() {
                return Err("`finally` is not supported yet".into());
            }
            let handler = try_stmt
                .handler
                .as_ref()
                .ok_or("`try` without `catch` is not supported yet")?;
            let catch_name = match &handler.param {
                Some(Pat::Ident(binding)) => binding.id.sym.to_string(),
                Some(_) => return Err("only a simple identifier catch binding is supported".into()),
                None => "_".to_string(),
            };
            let body = lower_stmts(&try_stmt.block.stmts)?;
            let catch_body = lower_stmts(&handler.body.stmts)?;
            Ok(vec![HirStmt::Try(body, catch_name, catch_body)])
        }

        other => Err(format!(
            "unsupported statement {other:?} (Phase 1 supports return/expr/let/if/while/for/throw/try)"
        )),
    }
}

fn lower_var_decl(var_decl: &swc_ecma_ast::VarDecl) -> Result<Vec<HirStmt>, String> {
    var_decl
        .decls
        .iter()
        .map(|decl| {
            let Pat::Ident(binding) = &decl.name else {
                return Err("only simple identifier bindings are supported in `let`/`const`".into());
            };
            let name = binding.id.sym.to_string();
            let init = decl
                .init
                .as_deref()
                .ok_or_else(|| format!("`{name}` needs an initializer"))?;
            let value = lower_expr(init)?;

            let ty = match &binding.type_ann {
                Some(ann) => lower_ts_type(&ann.type_ann)?,
                None => infer_literal_type(&value).ok_or_else(|| {
                    format!(
                        "cannot infer the type of `{name}`; add an explicit type annotation \
                         (Phase 1 only infers `let`/`const` types from a literal initializer)"
                    )
                })?,
            };

            Ok(HirStmt::Let(name, ty, value))
        })
        .collect()
}

/// Phase 1 has no general type inference; this only covers the trivial case
/// of a `let`/`const` initialized directly with a literal, so simple loop
/// counters and accumulators don't need a redundant type annotation.
fn infer_literal_type(expr: &HirExpr) -> Option<HirType> {
    match expr {
        HirExpr::Lit(HirLit::F64(_)) => Some(HirType::F64),
        HirExpr::Lit(HirLit::Str(_)) => Some(HirType::Str),
        HirExpr::Lit(HirLit::Bool(_)) => Some(HirType::Bool),
        HirExpr::ArrayLit(_) => Some(HirType::Array(Box::new(HirType::F64))),
        _ => None,
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

        Expr::Array(array_lit) => {
            let elems = array_lit
                .elems
                .iter()
                .map(|elem| match elem {
                    Some(e) if e.spread.is_none() => lower_expr(&e.expr),
                    Some(_) => Err("spread elements are not supported in array literals".to_string()),
                    None => Err("elisions are not supported in array literals".to_string()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(HirExpr::ArrayLit(elems))
        }

        Expr::Member(member) => lower_member_read(member),

        Expr::Assign(assign) => lower_assign(assign),

        Expr::Update(update) => lower_update(update),

        other => Err(format!(
            "unsupported expression {other:?} (Phase 1 supports literals, identifiers, binary ops, calls, arrays, indexing, assignment, ++/--)"
        )),
    }
}

fn lower_member_read(member: &swc_ecma_ast::MemberExpr) -> Result<HirExpr, String> {
    match &member.prop {
        MemberProp::Computed(computed) => {
            let obj = lower_expr(&member.obj)?;
            let index = lower_expr(&computed.expr)?;
            Ok(HirExpr::Index(Box::new(obj), Box::new(index)))
        }
        MemberProp::Ident(prop) if prop.sym == *"length" => {
            let obj = lower_expr(&member.obj)?;
            Ok(HirExpr::ArrayLen(Box::new(obj)))
        }
        _ => Err("unsupported property access (Phase 1 only supports `arr[i]` and `arr.length`)".into()),
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

/// An assignment target, resolved down to either a plain variable or an
/// array index -- the two shapes `lower_assign`/`lower_update` support.
enum Target {
    Var(Symbol),
    Index(HirExpr, HirExpr),
}

fn lower_assign_target(target: &AssignTarget) -> Result<Target, String> {
    let AssignTarget::Simple(simple) = target else {
        return Err("destructuring assignment targets are not supported".into());
    };
    match simple {
        SimpleAssignTarget::Ident(binding) => Ok(Target::Var(binding.id.sym.to_string())),
        SimpleAssignTarget::Member(member) => match &member.prop {
            MemberProp::Computed(computed) => Ok(Target::Index(
                lower_expr(&member.obj)?,
                lower_expr(&computed.expr)?,
            )),
            _ => Err("only `arr[i] = ...` member assignment is supported".into()),
        },
        _ => Err("unsupported assignment target".into()),
    }
}

fn target_to_read_expr(target: &Target) -> HirExpr {
    match target {
        Target::Var(name) => HirExpr::Var(name.clone()),
        Target::Index(arr, idx) => HirExpr::Index(Box::new(arr.clone()), Box::new(idx.clone())),
    }
}

fn build_assign(target: Target, value: HirExpr) -> HirExpr {
    match target {
        Target::Var(name) => HirExpr::Assign(name, Box::new(value)),
        Target::Index(arr, idx) => {
            HirExpr::IndexAssign(Box::new(arr), Box::new(idx), Box::new(value))
        }
    }
}

fn compound_op(op: AssignOp) -> Option<BinOp> {
    match op {
        AssignOp::AddAssign => Some(BinOp::Add),
        AssignOp::SubAssign => Some(BinOp::Sub),
        AssignOp::MulAssign => Some(BinOp::Mul),
        AssignOp::DivAssign => Some(BinOp::Div),
        _ => None,
    }
}

fn lower_assign(assign: &swc_ecma_ast::AssignExpr) -> Result<HirExpr, String> {
    let target = lower_assign_target(&assign.left)?;
    let rhs = lower_expr(&assign.right)?;

    let value = if assign.op == AssignOp::Assign {
        rhs
    } else if let Some(op) = compound_op(assign.op) {
        HirExpr::BinOp(op, Box::new(target_to_read_expr(&target)), Box::new(rhs))
    } else {
        return Err(format!(
            "unsupported compound assignment operator {:?}",
            assign.op
        ));
    };

    Ok(build_assign(target, value))
}

fn lower_update(update: &swc_ecma_ast::UpdateExpr) -> Result<HirExpr, String> {
    // Phase 1 always yields the *new* value (prefix semantics), even for
    // postfix `i++`/`i--`. This only matters when the expression's value is
    // used, which doesn't happen in the `for (...; ...; i++)` / bare
    // `i++;` forms this is meant to support.
    let target = match update.arg.as_ref() {
        Expr::Ident(ident) => Target::Var(ident.sym.to_string()),
        Expr::Member(member) => match &member.prop {
            MemberProp::Computed(computed) => {
                Target::Index(lower_expr(&member.obj)?, lower_expr(&computed.expr)?)
            }
            _ => return Err("unsupported ++/-- target".into()),
        },
        _ => return Err("unsupported ++/-- target".into()),
    };

    let op = match update.op {
        UpdateOp::PlusPlus => BinOp::Add,
        UpdateOp::MinusMinus => BinOp::Sub,
    };
    let one = HirExpr::Lit(HirLit::F64(1.0));
    let value = HirExpr::BinOp(op, Box::new(target_to_read_expr(&target)), Box::new(one));
    Ok(build_assign(target, value))
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
                return Err("spread arguments are not supported".to_string());
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
        let program = lower(r#"function main(): void { console.log("Hello, Thaw!"); }"#);
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

    #[test]
    fn desugars_classic_for_loop_into_let_and_while() {
        let program = lower(
            "function main(): void { for (let i = 0; i < 10; i = i + 1) { console.log(i); } }",
        );
        let f = &program.functions[0];
        assert_eq!(f.body.len(), 2);
        assert!(matches!(f.body[0], HirStmt::Let(ref n, HirType::F64, _) if n == "i"));
        let HirStmt::While(ref cond, ref body) = f.body[1] else {
            panic!("expected desugared while loop, got {:?}", f.body[1]);
        };
        assert_eq!(
            *cond,
            HirExpr::BinOp(
                BinOp::Lt,
                Box::new(HirExpr::Var("i".into())),
                Box::new(HirExpr::Lit(HirLit::F64(10.0))),
            )
        );
        // console.log(i) + the `i = i + 1` update appended to the body.
        assert_eq!(body.len(), 2);
        assert!(matches!(body[1], HirStmt::Expr(HirExpr::Assign(ref n, _)) if n == "i"));
    }

    #[test]
    fn lowers_array_literal_index_and_length() {
        let program = lower(
            "function main(): void { const xs: number[] = [1, 2, 3]; console.log(xs[1]); console.log(xs.length); }",
        );
        let f = &program.functions[0];
        assert_eq!(
            f.body[0],
            HirStmt::Let(
                "xs".into(),
                HirType::Array(Box::new(HirType::F64)),
                HirExpr::ArrayLit(vec![
                    HirExpr::Lit(HirLit::F64(1.0)),
                    HirExpr::Lit(HirLit::F64(2.0)),
                    HirExpr::Lit(HirLit::F64(3.0)),
                ]),
            )
        );
        assert_eq!(
            f.body[1],
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::Index(
                    Box::new(HirExpr::Var("xs".into())),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )],
            ))
        );
        assert_eq!(
            f.body[2],
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::ArrayLen(Box::new(HirExpr::Var("xs".into())))],
            ))
        );
    }

    #[test]
    fn lowers_try_catch() {
        let program = lower(
            r#"function main(): void {
                try {
                    throw "boom";
                } catch (e) {
                    console.log(e);
                }
            }"#,
        );
        let f = &program.functions[0];
        assert_eq!(
            f.body,
            vec![HirStmt::Try(
                vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str("boom".into())))],
                "e".into(),
                vec![HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("console.log".into())),
                    vec![HirExpr::Var("e".into())],
                ))],
            )]
        );
    }
}
