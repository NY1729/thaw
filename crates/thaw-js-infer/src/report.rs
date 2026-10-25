//! Driver, per-function classification and TypeScript annotation output.

use crate::analyze::An;
use crate::prescan::*;
use crate::ty::*;
use crate::Config;
use std::collections::{BTreeMap, HashSet};
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

#[derive(Debug, Clone)]
pub struct FnReport {
    pub name: String,
    pub lo: u32,
    pub class: Class,
    /// no `any` slot and no `any`-typed expression: could be lowered without any dynamic value
    pub closed: bool,
    pub reasons: Vec<Reason>,
    /// (slot name, TypeScript type, class); params, then locals
    pub slots: Vec<(String, String, Class)>,
    pub ret: String,
    pub exprs: usize,
    pub any_exprs: usize,
}

#[derive(Debug, Default, Clone)]
pub struct Summary {
    pub functions: usize,
    pub by_class: BTreeMap<Class, usize>,
    pub closed: usize,
    /// first `any` reason of each function
    pub fn_reason: BTreeMap<Reason, usize>,
    /// every `any` slot
    pub slot_reason: BTreeMap<Reason, usize>,
    pub slots: usize,
    pub slots_by_class: BTreeMap<Class, usize>,
    pub exprs: usize,
    pub any_exprs: usize,
    pub expr_reason: BTreeMap<Reason, usize>,
    pub passes: usize,
}

impl An {
    fn classify(&mut self, t: &Ty, seen: &mut HashSet<usize>) -> (Class, Option<Reason>) {
        let atoms = match t {
            Ty::Bottom => return (Class::Concrete, None),
            Ty::Any(r) => return (Class::Any, Some(*r)),
            Ty::Set(s) => s.clone(),
        };
        let (mut nonnull, mut nullish, mut worst) = (0, 0, Class::Concrete);
        for at in atoms {
            match at {
                Atom::Undef | Atom::Null => nullish += 1,
                Atom::Num | Atom::Str | Atom::Bool => nonnull += 1,
                Atom::Arr(s) | Atom::Obj(s) => {
                    nonnull += 1;
                    let s = self.find(s);
                    if !seen.insert(s) {
                        continue;
                    }
                    let inner: Vec<Ty> = match &self.sites[s] {
                        Site::Dead(r) => return (Class::Any, Some(*r)),
                        Site::Arr(e) => vec![e.clone()],
                        Site::Obj(m) => m.values().cloned().collect(),
                        Site::Link(_) => vec![],
                    };
                    for it in inner {
                        let (c, r) = self.classify(&it, seen);
                        if c == Class::Any {
                            return (c, r);
                        }
                        worst = worst.max(c);
                    }
                }
            }
        }
        let own = if nonnull >= 2 {
            Class::Union
        } else if nonnull == 1 && nullish > 0 {
            Class::Nullable
        } else {
            Class::Concrete
        };
        (own.max(worst), None)
    }

    fn ts(&mut self, t: &Ty, depth: usize) -> String {
        if depth > 3 {
            return "any".into();
        }
        match t {
            Ty::Bottom => "never".into(),
            Ty::Any(_) => "any".into(),
            Ty::Set(atoms) => {
                let mut parts = Vec::new();
                for at in atoms.clone() {
                    parts.push(match at {
                        Atom::Undef => "undefined".to_string(),
                        Atom::Null => "null".into(),
                        Atom::Num => "number".into(),
                        Atom::Str => "string".into(),
                        Atom::Bool => "boolean".into(),
                        Atom::Arr(s) => {
                            let s = self.find(s);
                            match self.sites[s].clone() {
                                Site::Arr(e) => {
                                    let inner = self.ts(&e, depth + 1);
                                    if inner.contains(' ') {
                                        format!("({inner})[]")
                                    } else {
                                        format!("{inner}[]")
                                    }
                                }
                                _ => "any[]".into(),
                            }
                        }
                        Atom::Obj(s) => {
                            let s = self.find(s);
                            match self.sites[s].clone() {
                                Site::Obj(m) => {
                                    let mut f = Vec::new();
                                    for (k, v) in m {
                                        let ident = k
                                            .chars()
                                            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                                            && !k.starts_with(|c: char| c.is_ascii_digit());
                                        let k = if ident { k } else { format!("{k:?}") };
                                        f.push(format!("{k}: {}", self.ts(&v, depth + 1)));
                                    }
                                    if f.is_empty() {
                                        "{}".into()
                                    } else {
                                        format!("{{ {} }}", f.join("; "))
                                    }
                                }
                                _ => "any".into(),
                            }
                        }
                    });
                }
                parts.sort_by_key(|p| p == "undefined" || p == "null"); // stable: nullish last
                parts.join(" | ")
            }
        }
    }
}

fn run(
    src: &str,
    cfg: Config,
) -> Result<
    (
        An,
        usize,
        thaw_parser::common::sync::Lrc<thaw_parser::common::SourceMap>,
        a::Module,
    ),
    String,
> {
    let (module, cm) = thaw_parser::parse_javascript_with_source_map(src)?;
    let mut st = State::default();
    st.funcs.push(FnInfo {
        name: "<top>".into(),
        ..Default::default()
    });
    {
        let mut p = Pre {
            st: &mut st,
            phase: 0,
            cur: vec![0],
            pending_name: None,
            list: 0,
            next_list: 0,
            direct: 0,
            stmt_assign: None,
        };
        module.visit_with(&mut p);
    }
    for (fid, f) in st.funcs.iter().enumerate() {
        for (n, k) in &f.declared {
            let ty = match k {
                DK::Fn | DK::SelfName => Ty::Any(Reason::Closure),
                DK::Class => Ty::Any(Reason::Class),
                _ => Ty::Bottom,
            };
            st.binds.insert(
                (fid, n.clone()),
                Bind {
                    ty,
                    ..Default::default()
                },
            );
        }
    }
    {
        let mut p = Pre {
            st: &mut st,
            phase: 1,
            cur: vec![0],
            pending_name: None,
            list: 0,
            next_list: 0,
            direct: 0,
            stmt_assign: None,
        };
        module.visit_with(&mut p);
    }
    for b in st.binds.values_mut() {
        let mut m = std::mem::take(&mut b.mentions);
        m.sort_by_key(|x| x.pos);
        b.definite = m.len() >= 2
            && m[0].kind == MK::DeclUninit
            && m[1].kind == MK::StmtAssign
            && m[0].list == m[1].list
            && m.iter().all(|x| !x.nested)
            && m[2..].iter().all(|x| x.pos >= m[1].rhs_end);
    }
    let known: Vec<((usize, String), usize)> = st
        .binds
        .iter()
        .filter(|(_, b)| b.n_assign == 1 && b.refs == b.callee_refs)
        .filter_map(|(k, b)| b.fn_value.map(|f| (k.clone(), f)))
        .collect();
    for (k, f) in known {
        st.funcs[f].name = k.1.clone();
        st.funcs[f].bind = Some(k);
    }
    let mut an = An::new(st, cfg);
    let mut passes = 0;
    while passes < 60 {
        passes += 1;
        an.changed = false;
        an.reset_counts();
        an.init_params();
        module.visit_with(&mut an);
        if !an.changed {
            break;
        }
    }
    Ok((an, passes, cm, module))
}

pub fn analyze_source(src: &str, cfg: Config) -> Result<(Vec<FnReport>, Summary), String> {
    let (mut an, passes, _, _) = run(src, cfg)?;
    let mut sum = Summary {
        passes,
        ..Default::default()
    };
    let mut out = Vec::new();
    for fid in 1..an.st.funcs.len() {
        let (name, lo, excluded, closed_world, nparams, params, mut locals, ret) = {
            let f = &an.st.funcs[fid];
            let excl = if f.async_ {
                Some(Reason::Async)
            } else if f.gen {
                Some(Reason::Generator)
            } else if f.uses_eval {
                Some(Reason::Eval)
            } else if f.uses_args {
                Some(Reason::Arguments)
            } else if f.uses_this && !f.arrow {
                Some(Reason::This)
            } else if f.exotic {
                Some(Reason::Exotic)
            } else {
                None
            };
            let params: Vec<String> = f
                .declared
                .iter()
                .filter(|(_, k)| **k == DK::Param)
                .map(|(n, _)| n.clone())
                .collect();
            let locals: Vec<String> = f
                .declared
                .iter()
                .filter(|(_, k)| matches!(**k, DK::Var | DK::Catch))
                .map(|(n, _)| n.clone())
                .collect();
            (
                f.name.clone(),
                f.lo,
                excl,
                f.bind.is_some() || f.iife,
                f.params.len(),
                params,
                locals,
                f.ret.clone(),
            )
        };
        locals.sort();
        let mut slots = Vec::new();
        let mut reasons = Vec::new();
        let mut class = Class::Concrete;
        let mut all_params_bottom = nparams > 0;
        let mut items: Vec<(String, Ty)> = Vec::new();
        for n in params.iter().chain(locals.iter()) {
            let t = an.st.binds[&(fid, n.clone())].ty.clone();
            items.push((n.clone(), t));
        }
        for (i, (n, t)) in items.iter().enumerate() {
            if i < params.len() && *t != Ty::Bottom {
                all_params_bottom = false;
            }
            let (c, r) = an.classify(t, &mut HashSet::new());
            let ts = an.ts(t, 0);
            slots.push((n.clone(), ts, c));
            if let Some(r) = r {
                reasons.push(r);
            }
            class = class.max(c);
            sum.slots += 1;
            *sum.slots_by_class.entry(c).or_default() += 1;
            if let Some(r) = r {
                *sum.slot_reason.entry(r).or_default() += 1;
            }
        }
        let (rc, rr) = an.classify(&ret, &mut HashSet::new());
        let ret_ts = an.ts(&ret, 0);
        class = class.max(rc);
        if let Some(r) = rr {
            reasons.push(r);
            *sum.slot_reason.entry(r).or_default() += 1;
        }
        sum.slots += 1;
        *sum.slots_by_class.entry(rc).or_default() += 1;
        if let Some(r) = excluded {
            class = Class::Any;
            reasons.insert(0, r);
        } else if closed_world && all_params_bottom && !params.is_empty() {
            class = Class::Dead;
        }
        let (exprs, any_exprs) = (an.st.funcs[fid].exprs, an.st.funcs[fid].any_exprs);
        let closed =
            matches!(class, Class::Concrete | Class::Nullable | Class::Union) && any_exprs == 0;
        *sum.by_class.entry(class).or_default() += 1;
        if closed {
            sum.closed += 1;
        }
        if class == Class::Any {
            if let Some(r) = reasons.first() {
                *sum.fn_reason.entry(*r).or_default() += 1;
            }
        }
        sum.functions += 1;
        sum.exprs += exprs;
        sum.any_exprs += any_exprs;
        out.push(FnReport {
            name,
            lo,
            class,
            closed,
            reasons,
            slots,
            ret: ret_ts,
            exprs,
            any_exprs,
        });
    }
    sum.expr_reason = an.any_hist.clone();
    Ok((out, sum))
}

struct Ann<'a> {
    st: &'a State,
    cur: Vec<usize>,
    /// (fid, name, ident lo, ident hi, arrow param needing parens)
    params: Vec<(usize, String, u32, u32)>,
    uninit: Vec<(usize, String, u32, bool)>,
}

impl<'a> Ann<'a> {
    fn fn_like(&mut self, f: F<'_>) {
        let Some(&fid) = self.st.by_span.get(&f.span()) else {
            return;
        };
        for p in f.pats() {
            let id = match p {
                a::Pat::Ident(i) => Some(i),
                a::Pat::Assign(asg) => match &*asg.left {
                    a::Pat::Ident(i) => Some(i),
                    _ => None,
                },
                _ => None,
            };
            if let Some(i) = id {
                self.params
                    .push((fid, i.id.sym.to_string(), i.span.lo.0, i.span.hi.0));
            }
        }
        self.cur.push(fid);
        for p in f.pats() {
            p.visit_with(self);
        }
        match f.body() {
            Body::Stmts(s) => s.iter().for_each(|x| x.visit_with(self)),
            Body::Expr(e) => e.visit_with(self),
            Body::None => {}
        }
        self.cur.pop();
    }
}

impl<'a> Visit for Ann<'a> {
    fn visit_function(&mut self, n: &a::Function) {
        self.fn_like(F::Func(n));
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        self.fn_like(F::Arrow(n));
    }
    fn visit_constructor(&mut self, n: &a::Constructor) {
        self.fn_like(F::Ctor(n));
    }
    fn visit_var_declarator(&mut self, n: &a::VarDeclarator) {
        if let a::Pat::Ident(i) = &n.name {
            // `true`: the initializer does not pin the type (none, `[]`, `{}`, `null`)
            let open = match n.init.as_deref().map(strip) {
                None | Some(a::Expr::Lit(a::Lit::Null(_))) => true,
                Some(a::Expr::Array(x)) => x.elems.is_empty(),
                Some(a::Expr::Object(x)) => x.props.is_empty(),
                _ => false,
            };
            self.uninit.push((
                *self.cur.last().unwrap(),
                i.id.sym.to_string(),
                i.span.hi.0,
                open,
            ));
        }
        n.visit_children_with(self);
    }
    fn visit_for_in_stmt(&mut self, n: &a::ForInStmt) {
        n.right.visit_with(self);
        n.body.visit_with(self);
    }
    fn visit_for_of_stmt(&mut self, n: &a::ForOfStmt) {
        n.right.visit_with(self);
        n.body.visit_with(self);
    }
}

/// Rewrite `src` so that every plain-identifier parameter and uninitialised `var` carries the
/// inferred TypeScript type (`any` when inference gave up). Initialised declarations are left to
/// the TS pipeline's own initializer inference.
pub fn annotate(src: &str, cfg: Config) -> Result<String, String> {
    let (mut an, _, cm, module) = run(src, cfg)?;
    let mut ann = Ann {
        st: &an.st,
        cur: vec![0],
        params: vec![],
        uninit: vec![],
    };
    module.visit_with(&mut ann);
    let (params, uninit) = (
        std::mem::take(&mut ann.params),
        std::mem::take(&mut ann.uninit),
    );
    let off = |p: u32| cm.lookup_byte_offset(thaw_parser::common::BytePos(p)).pos.0 as usize;
    let mut ins: Vec<(usize, String)> = Vec::new();
    let ty_of = |an: &mut An, fid: usize, n: &str| -> Option<String> {
        let t = an.st.binds.get(&(fid, n.to_string()))?.ty.clone();
        if t == Ty::Bottom {
            return Some("any".into());
        }
        let (c, _) = an.classify(&t, &mut HashSet::new());
        Some(if c == Class::Any {
            "any".into()
        } else {
            an.ts(&t, 0)
        })
    };
    for (fid, n, lo, hi) in params {
        let Some(t) = ty_of(&mut an, fid, &n) else {
            continue;
        };
        let before = src[..off(lo)].trim_end();
        if an.st.funcs[fid].arrow && !before.ends_with('(') && !before.ends_with(',') {
            ins.push((off(lo), "(".into()));
            ins.push((off(hi), format!(": {t})")));
        } else {
            ins.push((off(hi), format!(": {t}")));
        }
    }
    for (fid, n, hi, open) in uninit {
        if let Some(t) = ty_of(&mut an, fid, &n) {
            // an initialised declaration keeps the TS pipeline's own inference unless the
            // inferred type is wider than the initializer (a union) or the initializer is open
            if open || t.contains(" | ") {
                ins.push((off(hi), format!(": {t}")));
            }
        }
    }
    ins.sort_by_key(|x| std::cmp::Reverse(x.0));
    let mut out = src.to_string();
    for (p, s) in ins {
        out.insert_str(p, &s);
    }
    Ok(out)
}
