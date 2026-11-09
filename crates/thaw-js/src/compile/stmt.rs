//! Statement nodes (plan TJ150 section 4, PR1 subset: expression statements, empty statements, blocks,
//! `var`/`let`/`const` with identifier patterns, `return`).

use thaw_parser::ast as a;

use super::expr;
use super::gen::{self, reject, Gen, CG};
use super::{Flow, Frame, St, CR};
use crate::Interp;

fn mk_st<F>(f: F) -> St
where
    F: Fn(&mut Interp, &mut Frame) -> CR<Flow> + 'static,
{
    Box::new(f)
}

/// Runs the statements in order; the first one that completes abruptly ends the list.
fn seq(list: Vec<St>) -> St {
    mk_st(move |it, f| {
        for s in &list {
            match s(it, f)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    })
}

/// The statements of a function body (or of a block), in the current compile-time scope.
pub(crate) fn body(g: &mut Gen, stmts: &[a::Stmt]) -> CG<Vec<St>> {
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        out.push(compile(g, s)?);
    }
    Ok(out)
}

fn block(g: &mut Gen, stmts: &[a::Stmt]) -> CG<St> {
    g.push();
    let list = body(g, stmts);
    g.pop();
    Ok(seq(list?))
}

pub(crate) fn compile(g: &mut Gen, s: &a::Stmt) -> CG<St> {
    match s {
        a::Stmt::Expr(e) => {
            let x = expr::compile(g, &e.expr)?;
            Ok(mk_st(move |it, f| {
                x(it, f)?;
                Ok(Flow::Next)
            }))
        }
        a::Stmt::Empty(_) => Ok(mk_st(|_, _| Ok(Flow::Next))),
        a::Stmt::Block(b) => block(g, &b.stmts),
        a::Stmt::Return(r) => {
            let x = match &r.arg {
                Some(e) => expr::compile(g, e)?,
                None => gen::undefined(),
            };
            Ok(mk_st(move |it, f| {
                f.ret = x(it, f)?;
                Ok(Flow::Return)
            }))
        }
        a::Stmt::Decl(a::Decl::Var(v)) => var_decl(g, v),
        _ => reject(),
    }
}

/// `slots[k] = init`.
fn store(k: u32, init: super::Ex) -> St {
    mk_st(move |it, f| {
        let v = init(it, f)?;
        f.slots[k as usize] = v;
        Ok(Flow::Next)
    })
}

fn var_decl(g: &mut Gen, v: &a::VarDecl) -> CG<St> {
    let is_var = matches!(v.kind, a::VarDeclKind::Var);
    let mutable = !matches!(v.kind, a::VarDeclKind::Const);
    let mut steps = Vec::with_capacity(v.decls.len());
    for d in &v.decls {
        let a::Pat::Ident(b) = &d.name else {
            return reject();
        };
        let name = &b.id.sym;
        if is_var {
            // `var` is hoisted: its slot exists already, and a bare `var x;` does nothing
            let Some(l) = g.var_local(name) else {
                return reject();
            };
            if let Some(e) = &d.init {
                steps.push(store(l.slot, expr::compile(g, e)?));
            }
        } else {
            let init = match &d.init {
                Some(e) => expr::compile(g, e)?,
                None => gen::undefined(),
            };
            // declared after the initialiser: `let x = x` reads the outer binding
            let k = g.declare_block(name, mutable);
            steps.push(store(k, init));
        }
    }
    Ok(seq(steps))
}
