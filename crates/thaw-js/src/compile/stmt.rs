//! Statement nodes (plan TJ150 section 4). PR1: expression statements, empty statements, blocks,
//! `var`/`let`/`const` with identifier patterns, `return`. PR3: `if`, `while`, `do-while`, `for`, and
//! unlabeled `break`/`continue`.

use std::rc::Rc;

use thaw_parser::ast as a;

use super::expr;
use super::gen::{self, reject, Gen, CG};
use super::num::{self, Bl};
use super::pat;
use super::{bx, Ex, Flow, Frame, St, CR};
use crate::{Ctl, Interp, Value};

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
        // hoisted: bound by the prologue (`fn_decls`); the analyzer refuses every other position
        a::Stmt::Decl(a::Decl::Fn(_)) => Ok(mk_st(|_, _| Ok(Flow::Next))),
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
        // TJ156: `throw` produces the same `Ctl::Throw` the AST does, after the argument is evaluated
        a::Stmt::Throw(t) => {
            let x = expr::compile(g, &t.arg)?;
            Ok(mk_st(move |it, f| {
                let v = x(it, f)?;
                Err(Box::new(Ctl::Throw(v)))
            }))
        }
        a::Stmt::Break(b) => jump(g, b.label.is_some(), Flow::Break(0)),
        a::Stmt::Continue(c) => jump(g, c.label.is_some(), Flow::Continue(0)),
        a::Stmt::Switch(sw) => switch_stmt(g, sw),
        a::Stmt::Try(t) => try_stmt(g, t),
        a::Stmt::ForOf(fs) if !fs.is_await => for_each(g, &fs.left, &fs.right, &fs.body, true),
        a::Stmt::ForIn(fs) => for_each(g, &fs.left, &fs.right, &fs.body, false),
        _ => reject(),
    }
}

/// A loop body: `break`/`continue` are legal inside it.
fn loop_body(g: &mut Gen, s: &a::Stmt) -> CG<St> {
    g.loops += 1;
    g.breakables += 1;
    let r = compile(g, s);
    g.loops -= 1;
    g.breakables -= 1;
    r
}

/// `break`/`continue` without a label, inside a loop (`continue`) or a loop or `switch` (`break`) of this
/// function. Labels are blocked by the analyzer, and a stray one is rejected rather than compiled.
fn jump(g: &Gen, labeled: bool, flow: Flow) -> CG<St> {
    let enclosing = match flow {
        Flow::Break(_) => g.breakables,
        _ => g.loops,
    };
    if labeled || enclosing == 0 {
        return reject();
    }
    Ok(mk_st(move |_, _| Ok(flow)))
}

/// `switch`: the discriminant once, the tests in source order up to the first strict match, else the first
/// `default` (wherever it is); the bodies from there on, with fallthrough. Only `break` ends the switch. The
/// analyzer refuses lexical and function declarations in a case, so the interpreter's inner scope is
/// unobservable and no scope is modelled here.
fn switch_stmt(g: &mut Gen, sw: &a::SwitchStmt) -> CG<St> {
    g.breakables += 1;
    let r = switch_parts(g, sw);
    g.breakables -= 1;
    r
}

fn switch_parts(g: &mut Gen, sw: &a::SwitchStmt) -> CG<St> {
    let disc = expr::compile(g, &sw.discriminant)?;
    let mut tests: Vec<Option<Ex>> = Vec::with_capacity(sw.cases.len());
    let mut bodies: Vec<St> = Vec::with_capacity(sw.cases.len());
    for case in &sw.cases {
        tests.push(match &case.test {
            Some(t) => Some(expr::compile(g, t)?),
            None => None,
        });
        bodies.push(seq(body(g, &case.cons)?));
    }
    let default_at = sw.cases.iter().position(|c| c.test.is_none());
    Ok(mk_st(move |it, f| {
        let d = disc(it, f)?;
        let mut matched = None;
        for (i, t) in tests.iter().enumerate() {
            if let Some(t) = t {
                let tv = t(it, f)?;
                if crate::strict_equals(&d, &tv) {
                    matched = Some(i);
                    break;
                }
            }
        }
        if let Some(start) = matched.or(default_at) {
            for b in &bodies[start..] {
                match b(it, f)? {
                    Flow::Next => {}
                    // `break` ends the switch normally; `continue` and `return` belong to the enclosing code
                    Flow::Break(0) => return Ok(Flow::Next),
                    other => return Ok(other),
                }
            }
        }
        Ok(Flow::Next)
    }))
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

/// `x = init` for a scoped `var` x: assigned by name in the call's scope, as `exec_var` does.
fn assign_scoped(name: &str, init: super::Ex) -> St {
    let name: Rc<str> = name.into();
    mk_st(move |it, f| {
        let v = init(it, f)?;
        bx(it.assign_var(&f.scope, &name, v, false))?;
        Ok(Flow::Next)
    })
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
        let b = match &d.name {
            a::Pat::Ident(b) => b,
            pat => {
                let Some(e) = &d.init else { return reject() };
                let init = expr::compile(g, e)?;
                let mode = match (is_var, mutable) {
                    (true, _) => pat::Mode::Var,
                    (_, true) => pat::Mode::Let,
                    _ => pat::Mode::Const,
                };
                let bind = pat::compile(g, pat, mode)?;
                steps.push(mk_st(move |it, f| {
                    let v = init(it, f)?;
                    bind(it, f, v)?;
                    Ok(Flow::Next)
                }));
                continue;
            }
        };
        let name = &b.id.sym;
        if is_var {
            // `var` is hoisted: its slot (or its scoped binding, declared by the prologue) exists already,
            // and a bare `var x;` does nothing
            match g.var_local(name) {
                Some(l) => {
                    if let Some(e) = &d.init {
                        steps.push(store(l.slot, expr::named(e, name, expr::compile(g, e)?)));
                    }
                }
                None if g.is_scoped(name) => {
                    if let Some(e) = &d.init {
                        steps.push(assign_scoped(
                            name,
                            expr::named(e, name, expr::compile(g, e)?),
                        ));
                    }
                }
                None => return reject(),
            }
        } else {
            let init = match &d.init {
                Some(e) => expr::named(e, name, expr::compile(g, e)?),
                None => gen::undefined(),
            };
            // declared after the initialiser: `let x = x` reads the outer binding
            if g.is_scoped(name) {
                let mode = if mutable {
                    pat::Mode::Let
                } else {
                    pat::Mode::Const
                };
                let bind = pat::target(g, name, mode)?;
                steps.push(mk_st(move |it, f| {
                    let v = init(it, f)?;
                    bind(it, f, v)?;
                    Ok(Flow::Next)
                }));
                continue;
            }
            let k = g.declare_block(name, mutable);
            steps.push(store(k, init));
        }
    }
    Ok(seq(steps))
}

/// `try`/`catch`/`finally`, as the interpreter's `Stmt::Try` arm: the block, then the handler for a thrown value
/// (a `node:vm` timeout is not catchable), then the finalizer, whose own abrupt completion replaces the result.
fn try_stmt(g: &mut Gen, t: &a::TryStmt) -> CG<St> {
    let blk = block(g, &t.block.stmts)?;
    let handler = match &t.handler {
        Some(h) => Some(catch_clause(g, h)?),
        None => None,
    };
    let fin = match &t.finalizer {
        Some(b) => Some(block(g, &b.stmts)?),
        None => None,
    };
    Ok(mk_st(move |it, f| {
        let mut result = blk(it, f);
        if let (Err(e), Some(h), false) = (&result, &handler, it.dl_hit) {
            if matches!(**e, Ctl::Throw(_)) {
                let Err(e) = std::mem::replace(&mut result, Ok(Flow::Next)) else {
                    unreachable!()
                };
                let Ctl::Throw(v) = *e else { unreachable!() };
                result = h(it, f, v);
            }
        }
        if let Some(fin) = &fin {
            match fin(it, f)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        result
    }))
}

type Handler = Box<dyn Fn(&mut Interp, &mut Frame, Value) -> CR<Flow>>;

/// `catch (p) { body }`: the parameter and the body's names share one block scope.
fn catch_clause(g: &mut Gen, h: &a::CatchClause) -> CG<Handler> {
    g.push();
    let r = (|| {
        let bind = match &h.param {
            Some(p) => Some(pat::compile(g, p, pat::Mode::Let)?),
            None => None,
        };
        let body = seq(body(g, &h.body.stmts)?);
        Ok(Box::new(move |it: &mut Interp, f: &mut Frame, v: Value| {
            if let Some(b) = &bind {
                b(it, f, v)?;
            }
            body(it, f)
        }) as Handler)
    })();
    g.pop();
    r
}

/// `for (head of right) body` / `for (head in right) body`. The head's `let`/`const` live in a scope of their
/// own that starts after `right`; the interpreter's per-iteration scope is unobservable (captures are refused).
fn for_each(g: &mut Gen, head: &a::ForHead, right: &a::Expr, body: &a::Stmt, of: bool) -> CG<St> {
    let right = expr::compile(g, right)?;
    g.push();
    let r = (|| {
        let bind: pat::Bind = match head {
            a::ForHead::VarDecl(v) => match v.decls.as_slice() {
                [d] if d.init.is_none() => {
                    let mode = match v.kind {
                        a::VarDeclKind::Var => pat::Mode::Var,
                        a::VarDeclKind::Const => pat::Mode::Const,
                        _ => pat::Mode::Let,
                    };
                    pat::compile(g, &d.name, mode)?
                }
                _ => return reject(),
            },
            // `for (x of ...)`: an assignment to an existing name
            a::ForHead::Pat(p) => match &**p {
                a::Pat::Ident(id) => {
                    let place = expr::Place::of(g, &id.id);
                    Box::new(move |it, f, v| place.write(it, f, v))
                }
                _ => return reject(),
            },
            _ => return reject(),
        };
        let body = loop_body(g, body)?;
        Ok(if of {
            for_of_loop(right, bind, body)
        } else {
            for_in_loop(right, bind, body)
        })
    })();
    g.pop();
    r
}

fn for_in_loop(right: Ex, bind: pat::Bind, body: St) -> St {
    mk_st(move |it, f| {
        let target = right(it, f)?;
        let keys = it.for_in_keys(&target);
        for key in keys {
            bx(it.check_deadline())?;
            bind(it, f, Value::Str(key.into()))?;
            if let Some(out) = after_body(body(it, f)?) {
                return Ok(out);
            }
        }
        Ok(Flow::Next)
    })
}

fn for_of_loop(right: Ex, bind: pat::Bind, body: St) -> St {
    mk_st(move |it, f| {
        let iterable = right(it, f)?;
        // arrays, strings and the built-in collections (live for Array/Set/Map), nothing to close
        if let Some(mut items) = it.iter_items_live(&iterable) {
            while let Some(item) = items.next(it) {
                bx(it.check_deadline())?;
                bind(it, f, item)?;
                if let Some(out) = after_body(body(it, f)?) {
                    return Ok(out);
                }
            }
            return Ok(Flow::Next);
        }
        // a user-defined iterable or a generator: pulled one value at a time, closed on early exit
        let Some((iterator, next)) = bx(it.get_iterator(&iterable))? else {
            return bx(it.throw_error("TypeError", "value is not iterable"));
        };
        loop {
            bx(it.check_deadline())?;
            let result = bx(it.call(&next, &iterator, &[]))?;
            if !matches!(result, Value::Object(_)) {
                return bx(it.throw_error("TypeError", "Iterator result is not an object"));
            }
            if bx(it.get(&result, "done"))?.truthy() {
                return Ok(Flow::Next);
            }
            let item = bx(it.get(&result, "value"))?;
            let outcome = match bind(it, f, item) {
                Ok(()) => body(it, f).map(after_body),
                Err(e) => Err(e),
            };
            match outcome {
                Ok(None) => {}
                Ok(Some(out)) => {
                    it.close_iterator(&iterator);
                    return Ok(out);
                }
                Err(e) => {
                    it.close_iterator(&iterator);
                    return Err(e);
                }
            }
        }
    })
}

/// The function declarations at the top of a function body, as `hoist_functions` binds them before any statement
/// runs: a closure over the call's scope, stored in the name's slot (or its scoped binding).
pub(crate) fn fn_decls(g: &mut Gen, stmts: &[a::Stmt]) -> CG<Vec<St>> {
    let mut out = Vec::new();
    for s in stmts {
        let a::Stmt::Decl(a::Decl::Fn(f)) = s else {
            continue;
        };
        let name: Rc<str> = Rc::from(f.ident.sym.as_ref());
        let func = Rc::new(f.function.clone());
        let slot = match g.var_local(&name) {
            Some(l) => Some(l.slot),
            None if g.is_scoped(&name) => None,
            None => return reject(),
        };
        out.push(mk_st(move |it, fr| {
            it.cstats.closures_created += 1;
            let interned = it.intern(&name);
            let v = bx(it.function_value(Some(interned), &func, &fr.scope))?;
            match slot {
                Some(k) => fr.slots[k as usize] = v,
                None => bx(it.assign_var(&fr.scope, &name, v, false))?,
            }
            Ok(Flow::Next)
        }));
    }
    Ok(out)
}
