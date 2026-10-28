//! Flow-sensitive narrowing of variable types inside `if` / `?:` / `&&` / `||` / `while` and after
//! guard clauses (`if (typeof x !== 'string') return;`). Narrowing is applied only to bindings that
//! are never assigned from a nested function and not re-assigned inside the narrowed region, and
//! never crosses a function boundary.

use crate::analyze::An;
use crate::prescan::*;
use crate::ty::*;
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

#[derive(Clone, Debug)]
pub enum Flt {
    IsStr,
    IsNum,
    IsBool,
    IsUndef,
    IsNull,
    IsNullish,
    IsFunc,
    /// `typeof x === 'object'`: null, arrays, objects
    IsObj,
    /// `Array.isArray(x)`; the payload is a call-site key for the fresh array site
    IsArr(u32),
    IsInst((usize, String)),
    Truthy,
    EqStr(String),
    EqNum(i32),
}

pub type Cond = ((usize, String), Flt, bool);

struct AssignScan<'a> {
    name: &'a str,
    found: bool,
}

impl AssignScan<'_> {
    fn pat(&mut self, p: &a::Pat) {
        let mut ids = Vec::new();
        pat_idents(p, &mut ids);
        if ids.contains(&self.name) {
            self.found = true;
        }
    }
}

impl Visit for AssignScan<'_> {
    fn visit_assign_expr(&mut self, n: &a::AssignExpr) {
        match &n.left {
            a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(b)) if &*b.id.sym == self.name => {
                self.found = true
            }
            a::AssignTarget::Pat(_) => self.found = true, // ponytail: any destructuring assignment blocks narrowing
            _ => {}
        }
        n.visit_children_with(self);
    }
    fn visit_update_expr(&mut self, n: &a::UpdateExpr) {
        if matches!(strip(&n.arg), a::Expr::Ident(i) if &*i.sym == self.name) {
            self.found = true;
        }
        n.visit_children_with(self);
    }
    fn visit_var_declarator(&mut self, n: &a::VarDeclarator) {
        self.pat(&n.name);
        n.visit_children_with(self);
    }
    fn visit_catch_clause(&mut self, n: &a::CatchClause) {
        if let Some(p) = &n.param {
            self.pat(p);
        }
        n.visit_children_with(self);
    }
    fn visit_for_in_stmt(&mut self, n: &a::ForInStmt) {
        self.head(&n.left);
        n.visit_children_with(self);
    }
    fn visit_for_of_stmt(&mut self, n: &a::ForOfStmt) {
        self.head(&n.left);
        n.visit_children_with(self);
    }
}

impl AssignScan<'_> {
    fn head(&mut self, h: &a::ForHead) {
        match h {
            a::ForHead::Pat(p) => self.pat(p),
            a::ForHead::VarDecl(v) => v.decls.iter().for_each(|d| self.pat(&d.name)),
            _ => {}
        }
    }
}

pub fn assigns_in_stmt(s: &a::Stmt, name: &str) -> bool {
    let mut v = AssignScan { name, found: false };
    s.visit_with(&mut v);
    v.found
}

pub fn assigns_in_stmts(s: &[a::Stmt], name: &str) -> bool {
    s.iter().any(|x| assigns_in_stmt(x, name))
}

pub fn assigns_in_expr(e: &a::Expr, name: &str) -> bool {
    let mut v = AssignScan { name, found: false };
    e.visit_with(&mut v);
    v.found
}

/// The statement unconditionally leaves the enclosing block (return / throw / break / continue).
pub fn exits(s: &a::Stmt) -> bool {
    match s {
        a::Stmt::Return(_) | a::Stmt::Throw(_) | a::Stmt::Break(_) | a::Stmt::Continue(_) => true,
        a::Stmt::Block(b) => b.stmts.last().is_some_and(exits),
        a::Stmt::If(i) => i
            .alt
            .as_ref()
            .is_some_and(|alt| exits(&i.cons) && exits(alt)),
        _ => false,
    }
}

impl An {
    fn narrowable(&self, name: &str) -> Option<(usize, String)> {
        let k = self.st.resolve(*self.cur.last().unwrap(), name)?;
        let b = self.st.binds.get(&k)?;
        (!b.captured_assign && b.fn_value.is_none()).then_some(k)
    }

    pub(crate) fn narrow_lookup(&self, k: &(usize, String)) -> Option<Ty> {
        self.narrow
            .iter()
            .rev()
            .find(|(kk, _)| kk == k)
            .map(|(_, t)| t.clone())
    }

    /// Facts implied by `e` evaluating truthy (`truth`) or falsy.
    pub(crate) fn conds(&mut self, e: &a::Expr, truth: bool, out: &mut Vec<Cond>) {
        use a::BinaryOp as B;
        match strip(e) {
            a::Expr::Unary(u) if u.op == a::UnaryOp::Bang => self.conds(&u.arg, !truth, out),
            a::Expr::Bin(b) if b.op == B::LogicalAnd && truth => {
                self.conds(&b.left, true, out);
                self.conds(&b.right, true, out);
            }
            a::Expr::Bin(b) if b.op == B::LogicalOr && !truth => {
                self.conds(&b.left, false, out);
                self.conds(&b.right, false, out);
            }
            a::Expr::Bin(b) if matches!(b.op, B::EqEqEq | B::NotEqEq | B::EqEq | B::NotEq) => {
                let eq = matches!(b.op, B::EqEqEq | B::EqEq);
                let loose = matches!(b.op, B::EqEq | B::NotEq);
                let t = truth == eq;
                for (l, r) in [(&b.left, &b.right), (&b.right, &b.left)] {
                    if let a::Expr::Unary(u) = strip(l) {
                        if u.op == a::UnaryOp::TypeOf {
                            if let (a::Expr::Ident(x), a::Expr::Lit(a::Lit::Str(s))) =
                                (strip(&u.arg), strip(r))
                            {
                                let flt = match s.value.as_str().unwrap_or("") {
                                    "string" => Flt::IsStr,
                                    "number" => Flt::IsNum,
                                    "boolean" => Flt::IsBool,
                                    "undefined" => Flt::IsUndef,
                                    "object" => Flt::IsObj,
                                    "function" => Flt::IsFunc,
                                    _ => continue,
                                };
                                if let Some(k) = self.narrowable(&x.sym) {
                                    out.push((k, flt, t));
                                    return;
                                }
                            }
                        }
                    }
                    if let a::Expr::Ident(x) = strip(l) {
                        let flt = match strip(r) {
                            a::Expr::Lit(a::Lit::Null(_)) => {
                                Some(if loose { Flt::IsNullish } else { Flt::IsNull })
                            }
                            a::Expr::Ident(u)
                                if &*u.sym == "undefined"
                                    && self
                                        .st
                                        .resolve(*self.cur.last().unwrap(), "undefined")
                                        .is_none() =>
                            {
                                Some(if loose { Flt::IsNullish } else { Flt::IsUndef })
                            }
                            a::Expr::Unary(u) if u.op == a::UnaryOp::Void => {
                                Some(if loose { Flt::IsNullish } else { Flt::IsUndef })
                            }
                            a::Expr::Lit(a::Lit::Str(s)) if !loose => {
                                Some(Flt::EqStr(s.value.as_str().unwrap_or("").to_string()))
                            }
                            a::Expr::Lit(a::Lit::Num(n))
                                if !loose && n.value.fract() == 0.0 && n.value.abs() < 1e9 =>
                            {
                                Some(Flt::EqNum(n.value as i32))
                            }
                            _ => None,
                        };
                        if let (Some(flt), Some(k)) = (flt, self.narrowable(&x.sym)) {
                            out.push((k, flt, t));
                            return;
                        }
                    }
                }
            }
            a::Expr::Bin(b) if b.op == B::InstanceOf => {
                if let (a::Expr::Ident(x), a::Expr::Ident(c)) = (strip(&b.left), strip(&b.right)) {
                    let ck = self.st.resolve(*self.cur.last().unwrap(), &c.sym);
                    if let (Some(k), Some(ck)) = (self.narrowable(&x.sym), ck) {
                        if self.class_modelled(&ck) {
                            out.push((k, Flt::IsInst(ck), truth));
                        }
                    }
                }
            }
            a::Expr::Call(c) => {
                if let a::Callee::Expr(ce) = &c.callee {
                    if let a::Expr::Member(m) = strip(ce) {
                        if let (a::Expr::Ident(o), a::MemberProp::Ident(p)) =
                            (strip(&m.obj), &m.prop)
                        {
                            if &*o.sym == "Array"
                                && &*p.sym == "isArray"
                                && self
                                    .st
                                    .resolve(*self.cur.last().unwrap(), "Array")
                                    .is_none()
                            {
                                if let Some(x) = c.args.first().filter(|x| x.spread.is_none()) {
                                    if let a::Expr::Ident(x) = strip(&x.expr) {
                                        if let Some(k) = self.narrowable(&x.sym) {
                                            out.push((k, Flt::IsArr(c.span.lo.0), truth));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            a::Expr::Ident(x) => {
                if let Some(k) = self.narrowable(&x.sym) {
                    out.push((k, Flt::Truthy, truth));
                }
            }
            _ => {}
        }
    }

    /// Run `f` with the facts in `conds` applied to the variables that `region` does not assign.
    pub(crate) fn with_narrow<R>(
        &mut self,
        conds: Vec<Cond>,
        region_assigns: &dyn Fn(&str) -> bool,
        f: impl FnOnce(&mut An) -> R,
    ) -> R {
        let mark = self.narrow.len();
        self.push_narrow(conds, region_assigns);
        let r = f(self);
        self.narrow.truncate(mark);
        r
    }

    pub(crate) fn push_narrow(&mut self, conds: Vec<Cond>, region_assigns: &dyn Fn(&str) -> bool) {
        for (k, flt, pos) in conds {
            if region_assigns(&k.1) {
                continue;
            }
            let base = match self.narrow_lookup(&k) {
                Some(t) => t,
                None => self.st.binds[&k].ty.clone(),
            };
            let nt = self.apply(&flt, &base, pos);
            self.narrow.push((k, nt));
        }
    }

    fn atom_matches(&mut self, f: &Flt, at: Atom) -> bool {
        match (f, at) {
            (Flt::IsFunc, Atom::Fn(_)) => true,
            (Flt::IsStr, Atom::Str | Atom::SLit(_)) => true,
            (Flt::IsNum, Atom::Num | Atom::NLit(_)) => true,
            (Flt::IsBool, Atom::Bool) => true,
            (Flt::IsUndef, Atom::Undef) => true,
            (Flt::IsNull, Atom::Null) => true,
            (Flt::IsNullish, Atom::Undef | Atom::Null) => true,
            (Flt::IsObj, Atom::Null | Atom::Arr(_) | Atom::Obj(_)) => true,
            (Flt::IsObj, Atom::Ns(n)) => crate::builtins::is_ext(n),
            (Flt::IsArr(_), Atom::Arr(_)) => true,
            (Flt::IsInst(k), Atom::Obj(s)) => {
                let r = self.find(s);
                self.site_class_of(r) == Some(self.family_root(k))
            }
            (Flt::EqStr(v), Atom::SLit(i)) => self.lits[i as usize] == *v,
            (Flt::EqNum(v), Atom::NLit(n)) => n == *v,
            _ => false,
        }
    }

    pub(crate) fn apply(&mut self, f: &Flt, t: &Ty, pos: bool) -> Ty {
        let atoms = match t {
            Ty::Bottom => return Ty::Bottom,
            Ty::Any(r) => return self.apply_any(f, *r, pos),
            Ty::Set(s) => s.clone(),
        };
        let mut out = Vec::new();
        for at in atoms {
            match f {
                Flt::Truthy => {
                    let falsy_possible = match at {
                        Atom::Arr(_) | Atom::Obj(_) | Atom::Fn(_) | Atom::Ns(_) => false,
                        Atom::NLit(n) => n == 0,
                        Atom::SLit(i) => self.lits[i as usize].is_empty(),
                        _ => true,
                    };
                    let truthy_possible = !matches!(at, Atom::Undef | Atom::Null)
                        && !matches!(at, Atom::NLit(0))
                        && !matches!(at, Atom::SLit(i) if self.lits[i as usize].is_empty());
                    if (pos && truthy_possible) || (!pos && falsy_possible) {
                        out.push(at);
                    }
                }
                Flt::EqStr(_) | Flt::EqNum(_) if pos => {
                    if self.atom_matches(f, at) {
                        out.push(at);
                    } else if let (Flt::EqStr(v), Atom::Str) = (f, at) {
                        let id = self.intern(v);
                        out.push(Atom::SLit(id));
                    } else if let (Flt::EqNum(v), Atom::Num) = (f, at) {
                        out.push(Atom::NLit(*v));
                    }
                }
                // non-objects and other objects are never instances of the class
                Flt::IsInst(_) if pos => {
                    if self.atom_matches(f, at) {
                        out.push(at);
                    }
                }
                // other classes of the same hierarchy share the record: cannot rule the class out
                Flt::IsInst(k) if !pos && self.fam_size(k) > 1 => out.push(at),
                _ => {
                    if self.atom_matches(f, at) == pos {
                        out.push(at);
                    }
                }
            }
        }
        self.norm_pub(out)
    }

    fn apply_any(&mut self, f: &Flt, r: Reason, pos: bool) -> Ty {
        if !pos {
            return Ty::Any(r);
        }
        match f {
            Flt::IsStr => Ty::str(),
            Flt::IsNum => Ty::num(),
            Flt::IsBool => Ty::bool(),
            Flt::IsUndef => Ty::undef(),
            Flt::IsNull => Ty::one(Atom::Null),
            Flt::IsNullish => Ty::Set(vec![Atom::Undef, Atom::Null]),
            Flt::IsFunc => Ty::Any(Reason::Closure),
            Flt::IsObj | Flt::Truthy => Ty::Any(r),
            Flt::IsArr(k) => {
                let s = self.site_pub((*k, 6), Site::Arr(Ty::Any(Reason::Other)));
                Ty::one(Atom::Arr(s))
            }
            Flt::IsInst(k) => {
                let s = self.class_site_pub(k);
                Ty::one(Atom::Obj(s))
            }
            Flt::EqStr(v) => {
                let id = self.intern(v);
                Ty::one(Atom::SLit(id))
            }
            Flt::EqNum(v) => Ty::one(Atom::NLit(*v)),
        }
    }
}
