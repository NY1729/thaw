//! Scope discovery (phase 0) and reference / assignment / usage counting (phase 1).
//! Scoping is per function (block scopes are merged into their function: shadowing inside one
//! function only ever makes types wider, never wrong).

use crate::ty::*;
use std::collections::{BTreeMap, HashMap};
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;
use thaw_parser::common::Spanned;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DK {
    Param,
    Var,
    Fn,
    Class,
    Catch,
    SelfName,
}

#[derive(Default)]
pub struct FnInfo {
    pub parent: usize,
    pub name: String,
    pub lo: u32,
    pub declared: BTreeMap<String, DK>,
    /// simple identifier params in order (`None` = pattern / rest)
    pub params: Vec<Option<String>>,
    pub defaults: Vec<bool>,
    pub async_: bool,
    pub gen: bool,
    pub arrow: bool,
    pub exotic: bool,
    pub uses_this: bool,
    pub uses_args: bool,
    pub uses_eval: bool,
    pub iife: bool,
    /// the binding this function is the single value of
    pub bind: Option<(usize, String)>,
    pub ret: Ty,
    pub exprs: usize,
    pub any_exprs: usize,
    /// param name -> (numeric use, string use)
    pub guess: HashMap<String, (bool, bool)>,
}

#[derive(Default)]
pub struct Bind {
    pub ty: Ty,
    pub refs: u32,
    pub callee_refs: u32,
    pub n_assign: u32,
    pub fn_value: Option<usize>,
    pub mentions: Vec<Mention>,
    /// `var x;` is always followed by a sibling `x = e;` before any other mention: never observed
    /// as `undefined`.
    pub definite: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MK {
    DeclUninit,
    StmtAssign,
    Other,
}

#[derive(Clone, Copy, Debug)]
pub struct Mention {
    pub pos: u32,
    pub kind: MK,
    pub list: u32,
    pub nested: bool,
    pub rhs_end: u32,
}

pub enum F<'a> {
    Func(&'a a::Function),
    Arrow(&'a a::ArrowExpr),
    Ctor(&'a a::Constructor),
}

pub enum Body<'a> {
    Stmts(&'a [a::Stmt]),
    Expr(&'a a::Expr),
    None,
}

impl<'a> F<'a> {
    pub fn span(&self) -> (u32, u32) {
        let s = match self {
            F::Func(f) => f.span,
            F::Arrow(f) => f.span,
            F::Ctor(f) => f.span,
        };
        (s.lo.0, s.hi.0)
    }
    pub fn pats(&self) -> Vec<&'a a::Pat> {
        match self {
            F::Func(f) => f.params.iter().map(|p| &p.pat).collect(),
            F::Arrow(f) => f.params.iter().collect(),
            F::Ctor(f) => f
                .params
                .iter()
                .filter_map(|p| match p {
                    a::ParamOrTsParamProp::Param(p) => Some(&p.pat),
                    _ => None,
                })
                .collect(),
        }
    }
    pub fn body(&self) -> Body<'a> {
        match self {
            F::Func(f) => f
                .body
                .as_ref()
                .map_or(Body::None, |b| Body::Stmts(&b.stmts)),
            F::Arrow(f) => match &*f.body {
                a::ArrowFunctionBody::FunctionBody(b) => Body::Stmts(&b.stmts),
                a::ArrowFunctionBody::Expr(e) => Body::Expr(e),
            },
            F::Ctor(f) => f
                .body
                .as_ref()
                .map_or(Body::None, |b| Body::Stmts(&b.stmts)),
        }
    }
}

pub fn pat_idents<'a>(p: &'a a::Pat, out: &mut Vec<&'a str>) {
    match p {
        a::Pat::Ident(i) => out.push(&i.id.sym),
        a::Pat::Array(arr) => arr.elems.iter().flatten().for_each(|e| pat_idents(e, out)),
        a::Pat::Object(o) => {
            for pr in &o.props {
                match pr {
                    a::ObjectPatProp::KeyValue(kv) => pat_idents(&kv.value, out),
                    a::ObjectPatProp::Assign(asg) => out.push(&asg.key.sym),
                    a::ObjectPatProp::Rest(r) => pat_idents(&r.arg, out),
                }
            }
        }
        a::Pat::Rest(r) => pat_idents(&r.arg, out),
        a::Pat::Assign(asg) => pat_idents(&asg.left, out),
        _ => {}
    }
}

pub fn strip(mut e: &a::Expr) -> &a::Expr {
    while let a::Expr::Paren(p) = e {
        e = &p.expr;
    }
    e
}

#[derive(Default)]
pub struct State {
    pub funcs: Vec<FnInfo>,
    pub by_span: HashMap<(u32, u32), usize>,
    pub binds: HashMap<(usize, String), Bind>,
}

impl State {
    pub fn resolve(&self, mut fid: usize, name: &str) -> Option<(usize, String)> {
        loop {
            if self.funcs[fid].declared.contains_key(name) {
                return Some((fid, name.to_string()));
            }
            if fid == 0 {
                return None;
            }
            fid = self.funcs[fid].parent;
        }
    }
}

/// Phase 0 (`phase == 0`): create functions + declarations. Phase 1: refs/assigns/usage.
pub struct Pre<'s> {
    pub st: &'s mut State,
    pub phase: u8,
    pub cur: Vec<usize>,
    pub pending_name: Option<String>,
    pub list: u32,
    pub next_list: u32,
    pub direct: u32,
    /// (assign expr lo, rhs hi, list): this assignment is a statement of its own list
    pub stmt_assign: Option<(u32, u32, u32)>,
}

impl<'s> Pre<'s> {
    fn fid(&self) -> usize {
        *self.cur.last().unwrap()
    }
    fn fn_like(&mut self, f: F<'_>, is_async: bool, gen: bool, arrow: bool) {
        let key = f.span();
        if self.phase == 0 {
            let id = self.st.funcs.len();
            let mut info = FnInfo {
                parent: self.fid(),
                lo: key.0,
                async_: is_async,
                gen,
                arrow,
                exotic: matches!(f, F::Ctor(_)),
                ..Default::default()
            };
            for p in f.pats() {
                let (simple, dflt) = match p {
                    a::Pat::Ident(i) => (Some(i.id.sym.to_string()), false),
                    a::Pat::Assign(asg) => match &*asg.left {
                        a::Pat::Ident(i) => (Some(i.id.sym.to_string()), true),
                        _ => (None, true),
                    },
                    _ => (None, false),
                };
                info.params.push(simple);
                info.defaults.push(dflt);
                let mut ids = Vec::new();
                pat_idents(p, &mut ids);
                for n in ids {
                    info.declared.insert(n.to_string(), DK::Param);
                }
            }
            if let Some(n) = self.pending_name.take() {
                info.name = n.clone();
                info.declared.entry(n).or_insert(DK::SelfName);
            }
            self.st.by_span.insert(key, id);
            self.st.funcs.push(info);
        }
        let id = self.st.by_span[&key];
        self.pending_name = None;
        self.cur.push(id);
        // visit param defaults + body
        for p in f.pats() {
            p.visit_with(self);
        }
        match f.body() {
            Body::Stmts(s) => self.stmt_list(s),
            Body::Expr(e) => e.visit_with(self),
            Body::None => {}
        }
        self.cur.pop();
    }

    fn stmt_list(&mut self, s: &[a::Stmt]) {
        let saved = self.list;
        self.next_list += 1;
        self.list = self.next_list;
        for x in s {
            self.direct = x.span().lo.0;
            x.visit_with(self);
        }
        self.list = saved;
    }

    fn mention(&mut self, name: &str, pos: u32, kind: MK, rhs_end: u32) {
        let fid = self.fid();
        let Some(k) = self.st.resolve(fid, name) else {
            return;
        };
        let m = Mention {
            pos,
            kind,
            list: self.list,
            nested: k.0 != fid,
            rhs_end,
        };
        if let Some(b) = self.st.binds.get_mut(&k) {
            b.mentions.push(m);
        }
    }

    fn declare(&mut self, name: &str, k: DK) {
        let fid = self.fid();
        self.st.funcs[fid]
            .declared
            .entry(name.to_string())
            .or_insert(k);
    }

    fn flag_up(&mut self, f: impl Fn(&mut FnInfo), through_arrows_only: bool) {
        let mut fid = self.fid();
        loop {
            f(&mut self.st.funcs[fid]);
            if fid == 0 || (through_arrows_only && !self.st.funcs[fid].arrow) {
                break;
            }
            fid = self.st.funcs[fid].parent;
        }
    }

    fn bind_mut(&mut self, name: &str) -> Option<&mut Bind> {
        let k = self.st.resolve(self.fid(), name)?;
        self.st.binds.get_mut(&k)
    }

    fn ref_ident(&mut self, name: &str, callee: bool, pos: u32) {
        self.mention(name, pos, MK::Other, 0);
        if name == "arguments" && self.st.resolve(self.fid(), name).is_none() {
            self.flag_up(|f| f.uses_args = true, true);
        }
        if name == "eval" && callee {
            self.flag_up(|f| f.uses_eval = true, false);
        }
        if let Some(b) = self.bind_mut(name) {
            b.refs += 1;
            if callee {
                b.callee_refs += 1;
            }
        }
    }

    fn assigned(&mut self, name: &str, n: u32, fn_value: Option<usize>) {
        if let Some(b) = self.bind_mut(name) {
            b.n_assign += n;
            if let Some(f) = fn_value {
                b.fn_value = Some(f);
            }
        }
    }

    fn fn_of(&self, e: &a::Expr) -> Option<usize> {
        match strip(e) {
            a::Expr::Fn(f) => self
                .st
                .by_span
                .get(&(f.function.span.lo.0, f.function.span.hi.0))
                .copied(),
            a::Expr::Arrow(f) => self.st.by_span.get(&(f.span.lo.0, f.span.hi.0)).copied(),
            _ => None,
        }
    }

    fn param_of(&self, e: &a::Expr) -> Option<String> {
        if let a::Expr::Ident(i) = strip(e) {
            let fid = self.fid();
            if self.st.funcs[fid].declared.get(&*i.sym) == Some(&DK::Param) {
                return Some(i.sym.to_string());
            }
        }
        None
    }

    fn note(&mut self, e: &a::Expr, num: bool, s: bool) {
        if let Some(p) = self.param_of(e) {
            let fid = self.fid();
            let g = self.st.funcs[fid].guess.entry(p).or_default();
            g.0 |= num;
            g.1 |= s;
        }
    }
}

const STR_METHODS: &[&str] = &[
    "charCodeAt",
    "charAt",
    "toLowerCase",
    "toUpperCase",
    "trim",
    "substring",
    "substr",
    "padStart",
    "padEnd",
    "codePointAt",
    "localeCompare",
    "startsWith",
    "endsWith",
    "normalize",
];

impl<'s> Visit for Pre<'s> {
    fn visit_block_stmt(&mut self, n: &a::BlockStmt) {
        self.stmt_list(&n.stmts);
    }
    fn visit_switch_case(&mut self, n: &a::SwitchCase) {
        n.test.visit_with(self);
        self.stmt_list(&n.cons);
    }
    fn visit_stmt(&mut self, n: &a::Stmt) {
        if self.phase == 1 {
            let (asg, in_for) = match n {
                a::Stmt::Expr(e) => (Some(strip(&e.expr)), false),
                a::Stmt::For(f) => (
                    match &f.init {
                        Some(a::VarDeclOrExpr::Expr(e)) => Some(strip(e)),
                        _ => None,
                    },
                    true,
                ),
                _ => (None, false),
            };
            let _ = in_for;
            if let (Some(a::Expr::Assign(x)), true) = (asg, n.span().lo.0 == self.direct) {
                if x.op == a::AssignOp::Assign {
                    self.stmt_assign = Some((x.span.lo.0, x.right.span().hi.0, self.list));
                }
            }
        }
        n.visit_children_with(self);
    }
    fn visit_function(&mut self, n: &a::Function) {
        self.fn_like(F::Func(n), n.is_async, n.is_generator, false);
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        self.fn_like(F::Arrow(n), n.is_async, n.is_generator, true);
    }
    fn visit_constructor(&mut self, n: &a::Constructor) {
        self.fn_like(F::Ctor(n), false, false, false);
    }
    fn visit_fn_decl(&mut self, n: &a::FnDecl) {
        let name = n.ident.sym.to_string();
        if self.phase == 0 {
            self.declare(&name, DK::Fn);
            self.pending_name = None;
        } else {
            let f = self
                .st
                .by_span
                .get(&(n.function.span.lo.0, n.function.span.hi.0))
                .copied();
            self.assigned(&name, 1, f);
        }
        n.function.visit_with(self);
        if self.phase == 0 {
            let id = self.st.by_span[&(n.function.span.lo.0, n.function.span.hi.0)];
            self.st.funcs[id].name = name;
        }
    }
    fn visit_fn_expr(&mut self, n: &a::FnExpr) {
        if self.phase == 0 {
            self.pending_name = n.ident.as_ref().map(|i| i.sym.to_string());
        }
        n.function.visit_with(self);
    }
    fn visit_class_decl(&mut self, n: &a::ClassDecl) {
        if self.phase == 0 {
            self.declare(&n.ident.sym, DK::Class);
        }
        n.class.visit_with(self);
    }
    fn visit_catch_clause(&mut self, n: &a::CatchClause) {
        if self.phase == 0 {
            if let Some(p) = &n.param {
                let mut ids = Vec::new();
                pat_idents(p, &mut ids);
                for i in ids {
                    self.declare(i, DK::Catch);
                }
            }
        } else if let Some(p) = &n.param {
            let mut ids = Vec::new();
            pat_idents(p, &mut ids);
            for i in ids {
                self.assigned(i, 2, None);
            }
        }
        n.body.visit_with(self);
    }
    fn visit_var_declarator(&mut self, n: &a::VarDeclarator) {
        let mut ids = Vec::new();
        pat_idents(&n.name, &mut ids);
        if self.phase == 0 {
            for i in &ids {
                self.declare(i, DK::Var);
            }
        } else if let a::Pat::Ident(i) = &n.name {
            if let Some(init) = &n.init {
                let f = self.fn_of(init);
                self.assigned(&i.id.sym, 1, f);
                self.mention(&i.id.sym, i.span.lo.0, MK::Other, 0);
            } else {
                self.mention(&i.id.sym, i.span.lo.0, MK::DeclUninit, 0);
            }
        } else {
            for i in &ids {
                self.assigned(i, 2, None);
            }
        }
        n.init.visit_with(self);
    }
    fn visit_for_in_stmt(&mut self, n: &a::ForInStmt) {
        self.heads(&n.left);
        n.right.visit_with(self);
        n.body.visit_with(self);
    }
    fn visit_for_of_stmt(&mut self, n: &a::ForOfStmt) {
        self.heads(&n.left);
        n.right.visit_with(self);
        n.body.visit_with(self);
    }
    fn visit_this_expr(&mut self, _: &a::ThisExpr) {
        if self.phase == 1 {
            self.flag_up(|f| f.uses_this = true, true);
        }
    }
    fn visit_with_stmt(&mut self, n: &a::WithStmt) {
        if self.phase == 1 {
            self.flag_up(|f| f.uses_eval = true, false);
        }
        n.visit_children_with(self);
    }
    fn visit_expr(&mut self, e: &a::Expr) {
        if self.phase == 0 {
            return e.visit_children_with(self);
        }
        match e {
            a::Expr::Ident(i) => self.ref_ident(&i.sym, false, i.span.lo.0),
            a::Expr::Call(c) => {
                if let a::Callee::Expr(ce) = &c.callee {
                    match strip(ce) {
                        a::Expr::Ident(i) => {
                            self.ref_ident(&i.sym, true, i.span.lo.0);
                            c.args.visit_with(self);
                            return;
                        }
                        a::Expr::Fn(_) | a::Expr::Arrow(_) => {
                            if let Some(f) = self.fn_of(ce) {
                                self.st.funcs[f].iife = true;
                            }
                        }
                        a::Expr::Member(m) => {
                            if let a::MemberProp::Ident(p) = &m.prop {
                                if STR_METHODS.contains(&&*p.sym) {
                                    self.note(&m.obj, false, true);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                e.visit_children_with(self)
            }
            a::Expr::Bin(b) => {
                use a::BinaryOp::*;
                let lit = |x: &a::Expr| match strip(x) {
                    a::Expr::Lit(a::Lit::Num(_)) => 1,
                    a::Expr::Lit(a::Lit::Str(_)) | a::Expr::Tpl(_) => 2,
                    _ => 0,
                };
                match b.op {
                    Sub | Mul | Div | Mod | Exp | BitAnd | BitOr | BitXor | LShift | RShift
                    | ZeroFillRShift => {
                        self.note(&b.left, true, false);
                        self.note(&b.right, true, false);
                    }
                    Add => {
                        if lit(&b.right) == 2 {
                            self.note(&b.left, false, true);
                        }
                        if lit(&b.left) == 2 {
                            self.note(&b.right, false, true);
                        }
                    }
                    Lt | Gt | LtEq | GtEq | EqEq | EqEqEq | NotEq | NotEqEq => {
                        for (x, y) in [(&b.left, &b.right), (&b.right, &b.left)] {
                            match lit(y) {
                                1 => self.note(x, true, false),
                                2 => self.note(x, false, true),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                e.visit_children_with(self)
            }
            a::Expr::Unary(u) => {
                if matches!(
                    u.op,
                    a::UnaryOp::Plus | a::UnaryOp::Minus | a::UnaryOp::Tilde
                ) {
                    self.note(&u.arg, true, false);
                }
                e.visit_children_with(self)
            }
            a::Expr::Assign(asg) => {
                match &asg.left {
                    a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(bi)) => {
                        let plain = asg.op == a::AssignOp::Assign;
                        match self.stmt_assign {
                            Some((lo, hi, list)) if lo == asg.span.lo.0 && plain => {
                                let saved = self.list;
                                self.list = list;
                                self.mention(&bi.id.sym, bi.span.lo.0, MK::StmtAssign, hi);
                                self.list = saved;
                            }
                            _ => self.mention(&bi.id.sym, bi.span.lo.0, MK::Other, 0),
                        }
                        let f = if plain { self.fn_of(&asg.right) } else { None };
                        self.assigned(&bi.id.sym, if plain { 1 } else { 2 }, f);
                    }
                    a::AssignTarget::Pat(p) => {
                        let mut ids = Vec::new();
                        match p {
                            a::AssignTargetPat::Array(x) => {
                                a::Pat::Array(x.clone()).visit_with(&mut Collect(&mut ids))
                            }
                            a::AssignTargetPat::Object(x) => {
                                a::Pat::Object(x.clone()).visit_with(&mut Collect(&mut ids))
                            }
                            _ => {}
                        }
                        for i in ids {
                            self.assigned(&i, 2, None);
                        }
                    }
                    _ => {}
                }
                e.visit_children_with(self)
            }
            a::Expr::Update(u) => {
                if let a::Expr::Ident(i) = strip(&u.arg) {
                    self.assigned(&i.sym, 2, None);
                }
                e.visit_children_with(self)
            }
            _ => e.visit_children_with(self),
        }
    }
}

struct Collect<'a>(&'a mut Vec<String>);
impl<'a> Visit for Collect<'a> {
    fn visit_binding_ident(&mut self, n: &a::BindingIdent) {
        self.0.push(n.id.sym.to_string());
    }
    fn visit_assign_pat_prop(&mut self, n: &a::AssignPatProp) {
        self.0.push(n.key.sym.to_string());
    }
}

impl<'s> Pre<'s> {
    fn heads(&mut self, h: &a::ForHead) {
        let mut ids = Vec::new();
        match h {
            a::ForHead::VarDecl(v) => {
                for d in &v.decls {
                    let mut x = Vec::new();
                    pat_idents(&d.name, &mut x);
                    ids.extend(x.into_iter().map(String::from));
                }
            }
            a::ForHead::Pat(p) => {
                let mut x = Vec::new();
                pat_idents(p, &mut x);
                ids.extend(x.into_iter().map(String::from));
            }
            _ => {}
        }
        for i in ids {
            if self.phase == 0 {
                if matches!(h, a::ForHead::VarDecl(_)) {
                    self.declare(&i, DK::Var);
                }
            } else {
                self.assigned(&i, 2, None);
            }
        }
    }
}
