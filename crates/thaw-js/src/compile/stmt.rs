//! Statement nodes (plan TJ150 section 4). PR1: expression statements, empty statements, blocks,
//! `var`/`let`/`const` with identifier patterns, `return`. PR3: `if`, `while`, `do-while`, `for`, and
//! unlabeled `break`/`continue`.

use thaw_parser::ast as a;

use super::expr;
use super::gen::{self, reject, Gen, CG};
use super::num::{self, Bl};
use super::{bx, Ex, Flow, Frame, St, CR};
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
            let x = num::discard(g, &e.expr)?;
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
        a::Stmt::If(i) => if_stmt(g, i),
        a::Stmt::While(w) => while_stmt(g, w),
        a::Stmt::DoWhile(w) => do_while_stmt(g, w),
        a::Stmt::For(fs) => for_stmt(g, fs),
        a::Stmt::Break(b) => jump(g, b.label.is_some(), Flow::Break(0)),
        a::Stmt::Continue(c) => jump(g, c.label.is_some(), Flow::Continue(0)),
        _ => reject(),
    }
}

/// A loop body: `break`/`continue` are legal inside it.
fn loop_body(g: &mut Gen, s: &a::Stmt) -> CG<St> {
    g.loops += 1;
    let r = compile(g, s);
    g.loops -= 1;
    r
}

/// `break`/`continue` without a label, inside a loop of this function. Labels are blocked by the analyzer,
/// and a stray one is rejected rather than compiled.
fn jump(g: &Gen, labeled: bool, flow: Flow) -> CG<St> {
    if labeled || g.loops == 0 {
        return reject();
    }
    Ok(mk_st(move |_, _| Ok(flow)))
}

/// What a loop does with one completed body: `None` to run the next iteration (normal or `continue`),
/// `Some(flow)` to leave the loop with `flow` (`break` ends it normally; anything else propagates).
fn after_body(fl: Flow) -> Option<Flow> {
    match fl {
        Flow::Next | Flow::Continue(0) => None,
        Flow::Break(0) => Some(Flow::Next),
        other => Some(other),
    }
}

fn if_stmt(g: &mut Gen, i: &a::IfStmt) -> CG<St> {
    let test = num::test(g, &i.test)?;
    let cons = compile(g, &i.cons)?;
    let alt = match &i.alt {
        Some(s) => Some(compile(g, s)?),
        None => None,
    };
    Ok(mk_st(move |it, f| {
        if test(it, f)? {
            cons(it, f)
        } else if let Some(other) = &alt {
            other(it, f)
        } else {
            Ok(Flow::Next)
        }
    }))
}

fn while_stmt(g: &mut Gen, w: &a::WhileStmt) -> CG<St> {
    let test: Bl = num::test(g, &w.test)?;
    let body = loop_body(g, &w.body)?;
    Ok(mk_st(move |it, f| {
        while test(it, f)? {
            // the interpreter's order: test, then the deadline check, then the body
            bx(it.check_deadline())?;
            if let Some(out) = after_body(body(it, f)?) {
                return Ok(out);
            }
        }
        Ok(Flow::Next)
    }))
}

fn do_while_stmt(g: &mut Gen, w: &a::DoWhileStmt) -> CG<St> {
    let body = loop_body(g, &w.body)?;
    let test: Bl = num::test(g, &w.test)?;
    Ok(mk_st(move |it, f| loop {
        bx(it.check_deadline())?;
        if let Some(out) = after_body(body(it, f)?) {
            return Ok(out);
        }
        if !test(it, f)? {
            return Ok(Flow::Next);
        }
    }))
}

fn for_stmt(g: &mut Gen, fs: &a::ForStmt) -> CG<St> {
    // the head's `let`/`const` live in their own block scope, as the interpreter's `cur` scope does
    g.push();
    let r = for_parts(g, fs);
    g.pop();
    r
}

fn for_parts(g: &mut Gen, fs: &a::ForStmt) -> CG<St> {
    let init: Option<St> = match &fs.init {
        Some(a::VarDeclOrExpr::VarDecl(v)) => Some(var_decl(g, v)?),
        Some(a::VarDeclOrExpr::Expr(e)) => {
            let x = expr::compile(g, e)?;
            Some(mk_st(move |it, f| {
                x(it, f)?;
                Ok(Flow::Next)
            }))
        }
        None => None,
    };
    let test: Option<Bl> = match &fs.test {
        Some(e) => Some(num::test(g, e)?),
        None => None,
    };
    let update: Option<Ex> = match &fs.update {
        Some(e) => Some(num::discard(g, e)?),
        None => None,
    };
    let body = loop_body(g, &fs.body)?;
    Ok(mk_st(move |it, f| {
        if let Some(i) = &init {
            i(it, f)?;
        }
        loop {
            bx(it.check_deadline())?;
            if let Some(t) = &test {
                if !t(it, f)? {
                    return Ok(Flow::Next);
                }
            }
            if let Some(out) = after_body(body(it, f)?) {
                return Ok(out);
            }
            if let Some(u) = &update {
                u(it, f)?;
            }
        }
    }))
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
