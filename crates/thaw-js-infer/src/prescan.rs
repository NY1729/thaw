//! Scope discovery (phase 0) and reference / assignment / usage counting (phase 1).
//! Scoping is per function (block scopes are merged into their function: shadowing inside one
//! function only ever makes types wider, never wrong).

use crate::ty::*;
use std::collections::{BTreeMap, HashMap};
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;
use thaw_parser::common::Spanned;

#[derive(Default, Debug)]
pub struct ClassInfo {
    pub ctor: Option<usize>,
    pub methods: HashMap<String, usize>,
    pub es: bool,
    /// the class value itself is used other than `new X` / `X.prototype`
    pub escaped: bool,
}

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
    pub hi: u32,
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
    /// a function literal passed directly as a call argument
    pub cb_arg: bool,
    /// name this function is known by (declaration, `exports.x =`, `key: function`, class method)
    pub prop_name: Option<String>,
    /// owning class / constructor (`X.prototype.m =`, `class X { m(){} }`)
    pub owner: Option<String>,
    /// instance method (`X.prototype.m =` / non-static class method): `this` is an instance of `owner`
    pub is_proto: bool,
    /// ES class constructor
    pub is_ctor: bool,
    /// class method / accessor / object-literal method: reachable without a value of its own
    pub member: bool,
    pub owner_key: Option<(usize, String)>,
    /// spans of `this.f = e;` statements that are direct statements of the body
    pub direct_this: HashMap<u32, String>,
    /// (factory key, exported name) under which this function is exported (`<default>` for `module.exports = f`)
    pub exports: Vec<(String, String)>,
    /// which parameters were typed from a trusted source (`TRUST_*` bits)
    pub trust: u8,
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
    pub is_const: bool,
    /// `var x = require('./y')`: the bundle factory it resolves to
    pub req: Option<String>,
    /// references that are a direct call argument (`arr.map(f)`)
    pub arg_refs: u32,
    /// assigned from inside a nested function: flow-sensitive narrowing must not trust it
    pub captured_assign: bool,
    pub new_refs: u32,
    pub proto_refs: u32,
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

impl FnInfo {
    /// A plain function declaration / literal / arrow: its value exists in the lattice (`Atom::Fn`).
    pub fn atomic(&self) -> bool {
        !self.member && !self.is_ctor && !self.exotic
    }
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
    /// Offset of the `{` of a block body (where a return type annotation goes before).
    pub fn block_span(&self) -> Option<(u32, u32)> {
        match self {
            F::Func(f) => f.body.as_ref().map(|b| (b.span.lo.0, b.span.hi.0)),
            F::Arrow(f) => match &*f.body {
                a::ArrowFunctionBody::FunctionBody(b) => Some((b.span.lo.0, b.span.hi.0)),
                _ => None,
            },
            F::Ctor(_) => None,
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

pub const TRUST_DTS: u8 = 1;
pub const TRUST_EVIDENCE: u8 = 2;
pub const TRUST_HINT: u8 = 4;

pub enum ExpTarget {
    Fn(usize),
    Bind((usize, String)),
}

#[derive(Default)]
pub struct State {
    pub bundle: Option<crate::bundle::Bundle>,
    pub span_factory: HashMap<(u32, u32), String>,
    pub export_assigns: Vec<(String, String, ExpTarget)>,
    pub classes: HashMap<(usize, String), ClassInfo>,
    /// classes we do not model (inheritance, `X.prototype = ...`, mixins, foreign constructors)
    pub complex: std::collections::HashSet<(usize, String)>,
    /// constructor / instance method function -> its modelled class
    pub fn_class: HashMap<usize, (usize, String)>,
    /// child class -> parent class (`extends`, `util.inherits`, `X.prototype = Object.create(Y.prototype)`)
    pub parents: HashMap<(usize, String), (usize, String)>,
    pub ok_creates: std::collections::HashSet<u32>,
    /// modelled class -> root of its hierarchy (instances of a hierarchy share one record)
    pub family: HashMap<(usize, String), (usize, String)>,
    /// root -> method name -> every method of that name in the hierarchy
    pub fam_methods: HashMap<(usize, String), HashMap<String, Vec<usize>>>,
    /// some class of the hierarchy is exported / used as a value
    pub fam_escaped: std::collections::HashSet<(usize, String)>,
    /// some class of the hierarchy is instantiated with `new` in the package
    pub fam_instantiated: std::collections::HashSet<(usize, String)>,
    pub fam_size: HashMap<(usize, String), usize>,
    /// (class, method) whose function value is read as a value: `this` may be rebound
    pub unsafe_methods: std::collections::HashSet<((usize, String), String)>,
    pub funcs: Vec<FnInfo>,
    pub by_span: HashMap<(u32, u32), usize>,
    pub binds: HashMap<(usize, String), Bind>,
}

impl State {
    /// Bundle factory (module) a function lives in; `""` outside a bundle.
    pub fn factory_of(&self, mut fid: usize) -> String {
        loop {
            let f = &self.funcs[fid];
            if let Some(k) = self.span_factory.get(&(f.lo, f.hi)) {
                return k.clone();
            }
            if fid == 0 {
                return String::new();
            }
            fid = f.parent;
        }
    }
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
    pub pending_prop: Option<(String, Option<String>)>,
    pub class_stack: Vec<Option<String>>,
    pub class_keys: Vec<Option<(usize, String)>>,
    pub pending_proto: bool,
    pub decl_const: bool,
    pub pending_member: bool,
    pub pending_owner_key: Option<(usize, String)>,
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
                hi: key.1,
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
            info.is_proto = std::mem::take(&mut self.pending_proto);
            info.owner_key = self.pending_owner_key.take();
            info.is_ctor = matches!(f, F::Ctor(_));
            info.member = std::mem::take(&mut self.pending_member);
            if let Some((n, o)) = self.pending_prop.take() {
                info.prop_name = Some(n);
                info.owner = o;
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
            Body::Stmts(s) => {
                if self.phase == 1 {
                    for st in s {
                        if let a::Stmt::Expr(es) = st {
                            if let a::Expr::Assign(asg) = strip(&es.expr) {
                                if let a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)) =
                                    &asg.left
                                {
                                    if matches!(strip(&m.obj), a::Expr::This(_)) {
                                        if let a::MemberProp::Ident(pn) = &m.prop {
                                            self.st.funcs[id]
                                                .direct_this
                                                .insert(asg.span.lo.0, pn.sym.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.stmt_list(s)
            }
            Body::Expr(e) => e.visit_with(self),
            Body::None => {}
        }
        self.cur.pop();
    }

    /// `require('lit')` -> the bundle factory it resolves to in the current module.
    fn require_target(&self, e: &a::Expr) -> Option<String> {
        let a::Expr::Call(c) = strip(e) else {
            return None;
        };
        let a::Callee::Expr(ce) = &c.callee else {
            return None;
        };
        if !matches!(strip(ce), a::Expr::Ident(i) if &*i.sym == "require") {
            return None;
        }
        let a::Expr::Lit(a::Lit::Str(s)) = &*c.args.first()?.expr else {
            return None;
        };
        let bundle = self.st.bundle.as_ref()?;
        let fk = self.st.factory_of(self.fid());
        bundle
            .require_maps
            .get(&fk)?
            .get(s.value.as_str()?)
            .cloned()
    }

    fn export_target(&self, e: &a::Expr) -> Option<ExpTarget> {
        match strip(e) {
            a::Expr::Fn(_) | a::Expr::Arrow(_) => self.fn_of(e).map(ExpTarget::Fn),
            a::Expr::Ident(i) => self.st.resolve(self.fid(), &i.sym).map(ExpTarget::Bind),
            _ => None,
        }
    }

    /// `exports.f = ...`, `module.exports.f = ...`, `module.exports = f | { f, g: function(){} }`
    fn record_exports(&mut self, asg: &a::AssignExpr) {
        if asg.op != a::AssignOp::Assign {
            return;
        }
        let a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)) = &asg.left else {
            return;
        };
        let is_module_exports = |e: &a::Expr| {
            matches!(strip(e), a::Expr::Member(mm)
                if matches!(strip(&mm.obj), a::Expr::Ident(o) if &*o.sym == "module")
                && matches!(&mm.prop, a::MemberProp::Ident(p) if &*p.sym == "exports"))
        };
        let fk = self.st.factory_of(self.fid());
        let named = matches!(strip(&m.obj), a::Expr::Ident(o) if matches!(&*o.sym, "exports" | "_exports"))
            || is_module_exports(&m.obj);
        if named {
            if let a::MemberProp::Ident(p) = &m.prop {
                if let Some(t) = self.export_target(&asg.right) {
                    self.st.export_assigns.push((fk, p.sym.to_string(), t));
                }
            }
        } else if matches!(strip(&m.obj), a::Expr::Ident(o) if &*o.sym == "module")
            && matches!(&m.prop, a::MemberProp::Ident(p) if &*p.sym == "exports")
        {
            if let Some(t) = self.export_target(&asg.right) {
                self.st
                    .export_assigns
                    .push((fk.clone(), crate::evidence::DEFAULT.to_string(), t));
            }
            if let a::Expr::Object(o) = strip(&asg.right) {
                for pr in &o.props {
                    let a::PropOrSpread::Prop(pr) = pr else {
                        continue;
                    };
                    let (name, val): (String, Option<ExpTarget>) = match &**pr {
                        a::Prop::Shorthand(i) => (
                            i.sym.to_string(),
                            self.st.resolve(self.fid(), &i.sym).map(ExpTarget::Bind),
                        ),
                        a::Prop::KeyValue(kv) => match &kv.key {
                            a::PropName::Ident(k) => {
                                (k.sym.to_string(), self.export_target(&kv.value))
                            }
                            _ => continue,
                        },
                        a::Prop::Method(mp) => match &mp.key {
                            a::PropName::Ident(k) => (
                                k.sym.to_string(),
                                self.st
                                    .by_span
                                    .get(&(mp.function.span.lo.0, mp.function.span.hi.0))
                                    .copied()
                                    .map(ExpTarget::Fn),
                            ),
                            _ => continue,
                        },
                        _ => continue,
                    };
                    if let Some(t) = val {
                        self.st.export_assigns.push((fk.clone(), name, t));
                    }
                }
            }
        }
    }

    /// `X` or (with `proto`) `X.prototype` -> the class binding.
    fn class_key_of(&self, e: &a::Expr, proto: bool) -> Option<(usize, String)> {
        let name = match strip(e) {
            a::Expr::Ident(i) if !proto => i.sym.to_string(),
            a::Expr::Member(m)
                if proto && matches!(&m.prop, a::MemberProp::Ident(q) if q.sym == "prototype") =>
            {
                match strip(&m.obj) {
                    a::Expr::Ident(i) => i.sym.to_string(),
                    _ => return None,
                }
            }
            _ => return None,
        };
        self.st.resolve(self.fid(), &name)
    }

    /// A reference that is part of declaring inheritance is not an escape of the class value.
    fn bump_proto(&mut self, k: &(usize, String)) {
        if let Some(b) = self.st.binds.get_mut(k) {
            b.proto_refs += 1;
        }
    }

    fn mark_complex(&mut self, e: &a::Expr) {
        let name = match strip(e) {
            a::Expr::Ident(i) => Some(i.sym.to_string()),
            a::Expr::Member(m) if matches!(&m.prop, a::MemberProp::Ident(q) if q.sym == "prototype") => {
                match strip(&m.obj) {
                    a::Expr::Ident(i) => Some(i.sym.to_string()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(k) = name.and_then(|n| self.st.resolve(self.fid(), &n)) {
            self.st.complex.insert(k);
        }
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
        let nested = self
            .st
            .resolve(self.fid(), name)
            .is_some_and(|k| k.0 != self.fid());
        if let Some(b) = self.bind_mut(name) {
            if nested {
                b.captured_assign = true;
            }
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
    fn visit_var_decl(&mut self, n: &a::VarDecl) {
        let saved = self.decl_const;
        self.decl_const = n.kind == a::VarDeclKind::Const;
        n.visit_children_with(self);
        self.decl_const = saved;
    }
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
        if self.phase == 0 {
            if let Some(Some(c)) = self.class_stack.last() {
                self.pending_prop = Some(("constructor".into(), Some(c.clone())));
                self.pending_owner_key = self.class_keys.last().cloned().flatten();
            }
        }
        self.fn_like(F::Ctor(n), false, false, false);
    }
    fn visit_fn_decl(&mut self, n: &a::FnDecl) {
        let name = n.ident.sym.to_string();
        if self.phase == 0 {
            self.declare(&name, DK::Fn);
            self.pending_name = None;
            self.pending_prop = Some((name.clone(), None));
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
        self.class_stack.push(Some(n.ident.sym.to_string()));
        self.class_keys
            .push(Some((self.fid(), n.ident.sym.to_string())));
        n.class.visit_with(self);
        self.class_keys.pop();
        self.class_stack.pop();
    }
    fn visit_class(&mut self, n: &a::Class) {
        if self.phase == 1 {
            if let Some(sup) = &n.super_class {
                let own = self.class_keys.last().cloned().flatten();
                match (own, self.class_key_of(sup, false)) {
                    (Some(c), Some(p)) => {
                        self.bump_proto(&p);
                        self.st.parents.insert(c, p);
                    }
                    (Some(c), None) => {
                        self.st.complex.insert(c);
                    }
                    _ => self.mark_complex(sup),
                }
            }
        }
        n.visit_children_with(self);
    }
    fn visit_class_expr(&mut self, n: &a::ClassExpr) {
        self.class_stack
            .push(n.ident.as_ref().map(|i| i.sym.to_string()));
        self.class_keys.push(None);
        n.class.visit_with(self);
        self.class_keys.pop();
        self.class_stack.pop();
    }
    fn visit_private_method(&mut self, n: &a::PrivateMethod) {
        self.pending_member = true;
        n.function.visit_with(self);
        self.pending_member = false;
    }
    fn visit_getter_prop(&mut self, n: &a::GetterProp) {
        self.pending_member = true;
        n.function.visit_with(self);
        self.pending_member = false;
    }
    fn visit_setter_prop(&mut self, n: &a::SetterProp) {
        self.pending_member = true;
        n.function.visit_with(self);
        self.pending_member = false;
    }
    fn visit_class_method(&mut self, n: &a::ClassMethod) {
        self.pending_member = true;
        if let a::PropName::Ident(k) = &n.key {
            let owner = self.class_stack.last().cloned().flatten();
            self.pending_prop = Some((k.sym.to_string(), owner));
            self.pending_proto = !n.is_static;
            self.pending_owner_key = self.class_keys.last().cloned().flatten();
        }
        n.key.visit_with(self);
        n.function.visit_with(self);
        self.pending_prop = None;
    }
    fn visit_method_prop(&mut self, n: &a::MethodProp) {
        self.pending_member = true;
        if let a::PropName::Ident(k) = &n.key {
            self.pending_prop = Some((k.sym.to_string(), None));
        }
        n.key.visit_with(self);
        n.function.visit_with(self);
        self.pending_prop = None;
        self.pending_member = false;
    }
    fn visit_key_value_prop(&mut self, n: &a::KeyValueProp) {
        if let (a::PropName::Ident(k), a::Expr::Fn(_) | a::Expr::Arrow(_)) =
            (&n.key, strip(&n.value))
        {
            self.pending_prop = Some((k.sym.to_string(), None));
        }
        n.key.visit_with(self);
        n.value.visit_with(self);
        self.pending_prop = None;
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
                if self.decl_const {
                    if let Some(b) = self.bind_mut(&i.id.sym) {
                        b.is_const = true;
                    }
                }
                if let Some(t) = self.require_target(init) {
                    if let Some(b) = self.bind_mut(&i.id.sym) {
                        b.req = Some(t);
                    }
                }
                self.mention(&i.id.sym, i.span.lo.0, MK::Other, 0);
            } else {
                self.mention(&i.id.sym, i.span.lo.0, MK::DeclUninit, 0);
            }
        } else {
            for i in &ids {
                self.assigned(i, 2, None);
            }
        }
        if let (0, a::Pat::Ident(i), Some(init)) = (self.phase, &n.name, &n.init) {
            if matches!(strip(init), a::Expr::Fn(_) | a::Expr::Arrow(_)) {
                self.pending_prop = Some((i.id.sym.to_string(), None));
            }
        }
        n.init.visit_with(self);
        self.pending_prop = None;
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
            if let a::Expr::Assign(asg) = e {
                if let (
                    a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)),
                    a::Expr::Fn(_) | a::Expr::Arrow(_),
                ) = (&asg.left, strip(&asg.right))
                {
                    if let a::MemberProp::Ident(p) = &m.prop {
                        self.pending_proto = matches!(strip(&m.obj), a::Expr::Member(pm) if matches!(&pm.prop, a::MemberProp::Ident(q) if q.sym == "prototype"));
                        let owner = match strip(&m.obj) {
                            a::Expr::Member(pm) if matches!(&pm.prop, a::MemberProp::Ident(q) if q.sym == "prototype") => {
                                match strip(&pm.obj) {
                                    a::Expr::Ident(c) => Some(c.sym.to_string()),
                                    _ => None,
                                }
                            }
                            a::Expr::Ident(c) if c.sym != "exports" && c.sym != "module" => {
                                Some(c.sym.to_string())
                            }
                            _ => None,
                        };
                        self.pending_prop = Some((p.sym.to_string(), owner));
                    }
                }
            }
            e.visit_children_with(self);
            if matches!(e, a::Expr::Assign(_)) {
                self.pending_prop = None;
            }
            return;
        }
        match e {
            a::Expr::Ident(i) => self.ref_ident(&i.sym, false, i.span.lo.0),
            a::Expr::New(n) => {
                if let a::Expr::Ident(i) = strip(&n.callee) {
                    self.ref_ident(&i.sym, false, i.span.lo.0);
                    if let Some(b) = self.bind_mut(&i.sym) {
                        b.new_refs += 1;
                    }
                    n.args.visit_with(self);
                } else {
                    e.visit_children_with(self)
                }
            }
            a::Expr::Member(m)
                if matches!(&m.prop, a::MemberProp::Ident(q) if q.sym == "prototype")
                    && matches!(strip(&m.obj), a::Expr::Ident(_)) =>
            {
                if let a::Expr::Ident(i) = strip(&m.obj) {
                    self.ref_ident(&i.sym, false, i.span.lo.0);
                    if let Some(b) = self.bind_mut(&i.sym) {
                        b.proto_refs += 1;
                    }
                }
            }
            a::Expr::Member(m)
                if matches!(&m.prop, a::MemberProp::Ident(_))
                    && matches!(strip(&m.obj), a::Expr::Member(pm)
                        if matches!(&pm.prop, a::MemberProp::Ident(q) if q.sym == "prototype")
                        && matches!(strip(&pm.obj), a::Expr::Ident(_))) =>
            {
                // `X.prototype.m` read as a value: the method may be called with another `this`
                if let (a::Expr::Member(pm), a::MemberProp::Ident(p)) = (strip(&m.obj), &m.prop) {
                    if let a::Expr::Ident(c) = strip(&pm.obj) {
                        if let Some(k) = self.st.resolve(self.fid(), &c.sym) {
                            self.st.unsafe_methods.insert((k, p.sym.to_string()));
                        }
                    }
                }
                e.visit_children_with(self)
            }
            a::Expr::Call(c) => {
                let cname = match &c.callee {
                    a::Callee::Expr(ce) => match strip(ce) {
                        a::Expr::Ident(i) => Some(i.sym.to_string()),
                        a::Expr::Member(m) => match &m.prop {
                            a::MemberProp::Ident(p) => Some(p.sym.to_string()),
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                };
                if matches!(cname.as_deref(), Some("inherits" | "__extends")) && c.args.len() == 2 {
                    let child = self.class_key_of(&c.args[0].expr, false);
                    let parent = self.class_key_of(&c.args[1].expr, false);
                    match (child, parent) {
                        (Some(ch), Some(pa)) => {
                            self.bump_proto(&ch);
                            self.bump_proto(&pa);
                            self.st.parents.insert(ch, pa);
                        }
                        _ => c.args.iter().for_each(|x| self.mark_complex(&x.expr)),
                    }
                } else if (cname.as_deref() == Some("create")
                    && !self.st.ok_creates.contains(&c.span.lo.0))
                    || matches!(
                        cname.as_deref(),
                        Some(
                            "inherits"
                                | "__extends"
                                | "extend"
                                | "setPrototypeOf"
                                | "assign"
                                | "mixin"
                                | "defineProperty"
                                | "defineProperties"
                        )
                    )
                {
                    for x in &c.args {
                        self.mark_complex(&x.expr);
                    }
                }
                // `A.call(this, ...)`: super-constructor call, not an escape of `A`
                if let a::Callee::Expr(ce) = &c.callee {
                    if let a::Expr::Member(mm) = strip(ce) {
                        if let (a::Expr::Ident(x), a::MemberProp::Ident(pn)) =
                            (strip(&mm.obj), &mm.prop)
                        {
                            if matches!(&*pn.sym, "call" | "apply")
                                && matches!(
                                    c.args.first().map(|a0| strip(&a0.expr)),
                                    Some(a::Expr::This(_))
                                )
                            {
                                self.ref_ident(&x.sym, true, x.span.lo.0);
                                c.args.visit_with(self);
                                return;
                            }
                        }
                    }
                }
                for x in &c.args {
                    if x.spread.is_none() {
                        if let Some(f) = self.fn_of(&x.expr) {
                            self.st.funcs[f].cb_arg = true;
                        }
                        if let a::Expr::Ident(i) = strip(&x.expr) {
                            if let Some(b) = self.bind_mut(&i.sym) {
                                b.arg_refs += 1;
                            }
                        }
                    }
                }
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
                self.record_exports(asg);
                match &asg.left {
                    a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)) if matches!(&m.prop, a::MemberProp::Ident(q) if q.sym == "prototype") =>
                    {
                        // `X.prototype = Object.create(Y.prototype)` is plain inheritance
                        let mut inherited = false;
                        if let a::Expr::Call(cc) = strip(&asg.right) {
                            if let a::Callee::Expr(ce) = &cc.callee {
                                let is_create = matches!(strip(ce), a::Expr::Member(mm)
                                    if matches!(&mm.prop, a::MemberProp::Ident(q) if q.sym == "create")
                                    && matches!(strip(&mm.obj), a::Expr::Ident(o) if &*o.sym == "Object"));
                                if is_create && cc.args.len() == 1 {
                                    if let (Some(ch), Some(pa)) = (
                                        self.class_key_of(&m.obj, false),
                                        self.class_key_of(&cc.args[0].expr, true),
                                    ) {
                                        self.bump_proto(&ch);
                                        self.st.parents.insert(ch, pa);
                                        self.st.ok_creates.insert(cc.span.lo.0);
                                        inherited = true;
                                    }
                                }
                            }
                        }
                        if !inherited {
                            self.mark_complex(&m.obj);
                        }
                    }
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
