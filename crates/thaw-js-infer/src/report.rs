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
    /// parameter / return types came from a `.d.ts` signature (trusted contract: needs an entry guard)
    pub dts: bool,
    /// which parameters came from a trusted source: `TRUST_DTS | TRUST_EVIDENCE | TRUST_HINT`
    pub trust: u8,
    /// `this` was typed as an instance of a modelled class (needs a brand / field-tag guard at entry)
    pub this_typed: bool,
    /// names under which the function is exported (`<default>` for `module.exports = f`)
    pub exports: Vec<String>,
    /// JavaScript predicate over the parameters (and `this`) under which the typed signature holds;
    /// `Some` for every function whose types rest on a trusted contract
    pub guard: Option<String>,
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
    /// functions matched to a `.d.ts` signature / of those, ones that ended up typed (not `any`)
    pub dts_matched: usize,
    pub dts_typed: usize,
    /// functions typed via a modelled `this` / of those, ones that ended up typed
    pub this_matched: usize,
    pub this_typed: usize,
    /// functions with evidence-typed parameters / of those, ones that ended up typed
    pub ev_matched: usize,
    pub ev_typed: usize,
    /// typed functions that rest on no trusted contract at all
    pub sound_typed: usize,
    /// typed functions that carry an entry guard
    pub guarded: usize,
    /// first `any` reason of `.d.ts`-matched functions that stayed `any`
    pub dts_reason: BTreeMap<Reason, usize>,
}

impl An {
    pub(crate) fn classify_pub(&mut self, t: &Ty) -> (Class, Option<Reason>) {
        self.classify(t, &mut HashSet::new())
    }
    pub(crate) fn ts_pub(&mut self, t: &Ty) -> String {
        self.ts(t, 0)
    }

    fn classify(&mut self, t: &Ty, seen: &mut HashSet<usize>) -> (Class, Option<Reason>) {
        let atoms = match t {
            Ty::Bottom => return (Class::Concrete, None),
            Ty::Any(r) => return (Class::Any, Some(*r)),
            Ty::Set(s) => s.clone(),
        };
        let (mut nonnull, mut nullish, mut worst) = (0, 0, Class::Concrete);
        let mut kinds = HashSet::new();
        for at in atoms {
            match at {
                Atom::Undef | Atom::Null => nullish += 1,
                Atom::Num | Atom::NLit(_) | Atom::Str | Atom::SLit(_) | Atom::Bool => {
                    let kind = match at {
                        Atom::Num | Atom::NLit(_) => 1,
                        Atom::Str | Atom::SLit(_) => 2,
                        _ => 3,
                    };
                    if kinds.insert(kind) {
                        nonnull += 1;
                    }
                }
                Atom::Ns(_) => {
                    if kinds.insert(5) {
                        nonnull += 1;
                    }
                }
                Atom::Fn(_) => {
                    if kinds.insert(4) {
                        nonnull += 1;
                    }
                }
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
                        Atom::Fn(_) => "Function".into(),
                        Atom::Ns(_) => "NodeModule".into(),
                        Atom::SLit(i) => format!("{:?}", self.lits[i as usize]),
                        Atom::NLit(n) => n.to_string(),
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
                            if let Some(k) = self.site_class_of(s) {
                                if !matches!(self.sites[s], Site::Dead(_)) {
                                    parts.push(k.1);
                                    continue;
                                }
                            }
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

/// Group constructors and instance methods by class; decide which classes we model.
fn link_classes(st: &mut State) {
    for fid in 1..st.funcs.len() {
        let f = &st.funcs[fid];
        let Some(owner) = f.owner.clone() else {
            continue;
        };
        if !(f.is_ctor || f.is_proto) {
            continue;
        }
        let key = f.owner_key.clone().or_else(|| st.resolve(f.parent, &owner));
        let Some(key) = key else { continue };
        let (is_ctor, pn) = (f.is_ctor, f.prop_name.clone());
        let ci = st.classes.entry(key).or_default();
        if is_ctor {
            ci.ctor = Some(fid);
            ci.es = true;
        } else if let Some(pn) = pn {
            ci.methods.insert(pn, fid);
        }
    }
    // classes that are only instantiated (`new F()`) or only take part in inheritance
    let mut extra: Vec<(usize, String)> = st
        .binds
        .iter()
        .filter(|(_, b)| b.new_refs > 0 && b.n_assign == 1 && b.fn_value.is_some())
        .map(|(k, _)| k.clone())
        .collect();
    for (c, p) in st.parents.clone() {
        extra.push(c);
        extra.push(p);
    }
    for k in extra {
        let is_es = st.binds.get(&k).is_some_and(|b| b.fn_value.is_none());
        let ci = st.classes.entry(k).or_default();
        if is_es {
            ci.es = true;
        }
    }
    let keys: Vec<_> = st.classes.keys().cloned().collect();
    for k in &keys {
        if !st.classes[k].es {
            let ctor = st
                .binds
                .get(k)
                .and_then(|b| if b.n_assign == 1 { b.fn_value } else { None });
            match ctor {
                Some(c) => st.classes.get_mut(k).unwrap().ctor = Some(c),
                None => {
                    st.complex_why
                        .entry(k.clone())
                        .or_insert("ctor-binding".into());
                    st.complex.insert(k.clone());
                }
            }
        }
    }
    // a hierarchy is modelled only as a whole
    loop {
        let mut changed = false;
        for (c, p) in st.parents.clone() {
            let bad = st.complex.contains(&c) || st.complex.contains(&p);
            if bad {
                changed |= st.complex.insert(c);
                changed |= st.complex.insert(p);
            }
        }
        if !changed {
            break;
        }
    }
    for k in &keys {
        if st.complex.contains(k) {
            continue;
        }
        let mut root = k.clone();
        let mut n = 0;
        while let Some(p) = st.parents.get(&root) {
            root = p.clone();
            n += 1;
            if n > 64 {
                st.complex.insert(k.clone());
                break;
            }
        }
        st.family.insert(k.clone(), root);
    }
    for k in keys {
        if st.complex.contains(&k) || !st.family.contains_key(&k) {
            continue;
        }
        let root = st.family[&k].clone();
        let b = &st.binds[&k];
        let escaped = b.refs > b.callee_refs + b.new_refs + b.proto_refs;
        let ci = st.classes.get_mut(&k).unwrap();
        ci.escaped = escaped;
        if escaped {
            st.fam_escaped.insert(root.clone());
        }
        if b.new_refs > 0 {
            st.fam_instantiated.insert(root.clone());
        }
        *st.fam_size.entry(root.clone()).or_default() += 1;
        let (ctor, methods): (Option<usize>, Vec<(String, usize)>) = (
            ci.ctor,
            ci.methods.iter().map(|(n, f)| (n.clone(), *f)).collect(),
        );
        if let Some(c) = ctor {
            st.fn_class.insert(c, k.clone());
            if !escaped {
                st.funcs[c].bind = Some(k.clone());
            }
        }
        for (name, m) in methods {
            st.fam_methods
                .entry(root.clone())
                .or_default()
                .entry(name.clone())
                .or_default()
                .push(m);
            if !st.unsafe_methods.contains(&(k.clone(), name)) {
                st.fn_class.insert(m, k.clone());
            }
        }
    }
}

pub(crate) fn run(
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
            pending_prop: None,
            class_stack: vec![],
            class_keys: vec![],
            pending_proto: false,
            decl_const: false,
            pending_member: false,
            pending_owner_key: None,
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
    st.bundle = crate::bundle::find(&module);
    if let Some(b) = &st.bundle {
        for (k, sp) in &b.factories {
            st.span_factory.insert(*sp, k.clone());
        }
    }
    {
        let mut p = Pre {
            st: &mut st,
            phase: 1,
            cur: vec![0],
            pending_name: None,
            pending_prop: None,
            class_stack: vec![],
            class_keys: vec![],
            pending_proto: false,
            decl_const: false,
            pending_member: false,
            pending_owner_key: None,
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
    // `module.exports = X` where X is a local object: its `X.m = function` members are the exports
    let mut default_objects: Vec<(String, (usize, String))> = Vec::new();
    for (fk, name, t) in std::mem::take(&mut st.export_assigns) {
        if name == crate::evidence::DEFAULT {
            if let ExpTarget::Bind(k) = &t {
                default_objects.push((fk.clone(), k.clone()));
            }
        }
        let f = match t {
            ExpTarget::Fn(f) => Some(f),
            ExpTarget::Bind(k) => {
                st.binds
                    .get(&k)
                    .and_then(|b| if b.n_assign == 1 { b.fn_value } else { None })
            }
        };
        if let Some(f) = f {
            st.funcs[f].exports.push((fk, name));
        }
    }
    // the bundle runtime calls every factory with its own arguments
    if let Some(b) = st.bundle.clone() {
        for (k, sp) in &b.factories {
            if let Some(&f) = st.by_span.get(sp) {
                st.funcs[f].exports.push((k.clone(), "<factory>".into()));
            }
        }
    }
    for fid in 1..st.funcs.len() {
        let f = &st.funcs[fid];
        let (Some(owner), Some(pn)) = (f.owner.clone(), f.prop_name.clone()) else {
            continue;
        };
        if f.is_proto || f.is_ctor {
            continue;
        }
        let Some(key) = st.resolve(f.parent, &owner) else {
            continue;
        };
        let hits: Vec<String> = default_objects
            .iter()
            .filter(|(_, k)| *k == key)
            .map(|(fk, _)| fk.clone())
            .collect();
        for fk in hits {
            st.funcs[fid].exports.push((fk, pn.clone()));
        }
    }
    let named_cbs: Vec<usize> = st
        .binds
        .values()
        .filter(|b| b.n_assign == 1 && b.arg_refs > 0 && b.refs == b.callee_refs + b.arg_refs)
        .filter_map(|b| b.fn_value)
        .collect();
    for f in named_cbs {
        st.funcs[f].cb_arg = true;
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
    link_classes(&mut st);
    // function declarations that are bound once evaluate to their own function value
    let decls: Vec<((usize, String), usize)> = st
        .funcs
        .iter()
        .enumerate()
        .flat_map(|(fid, f)| {
            f.declared
                .iter()
                .filter(|(_, k)| **k == DK::Fn)
                .map(move |(n, _)| (fid, n.clone()))
        })
        .filter_map(|k| {
            let b = st.binds.get(&k)?;
            (b.n_assign == 1)
                .then_some(b.fn_value)
                .flatten()
                .map(|f| (k, f))
        })
        .collect();
    for (k, f) in decls {
        st.binds.get_mut(&k).unwrap().ty = Ty::one(Atom::Fn(f as u32));
    }
    let mut an = An::new(st, cfg);
    an.init_dts();
    let mut passes = 0;
    // stage A runs without deciding `any` for exported functions (call-site evidence may still
    // arrive); stage B then finalises them
    while passes < 120 {
        passes += 1;
        an.changed = false;
        an.reset_counts();
        an.init_params();
        module.visit_with(&mut an);
        if !an.changed {
            if an.escape_final {
                break;
            }
            an.escape_final = true;
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
        let this_ok = an.this_ok(fid);
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
            } else if f.exotic && !this_ok {
                Some(Reason::Exotic)
            } else {
                None
            };
            let mut params: Vec<String> = f.params.iter().flatten().cloned().collect();
            for (n, k) in &f.declared {
                if *k == DK::Param && !params.contains(n) {
                    params.push(n.clone());
                }
            }
            let locals: Vec<String> = f
                .declared
                .iter()
                .filter(|(_, k)| matches!(**k, DK::Var | DK::Catch))
                .map(|(n, _)| n.clone())
                .collect();
            (
                if f.name.is_empty() {
                    f.prop_name.clone().unwrap_or_default()
                } else {
                    f.name.clone()
                },
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
        let dts_fn = an.dts_fns.contains_key(&fid);
        let ret = match (&ret, an.dts_ret.get(&fid)) {
            (Ty::Any(_), Some(d)) => d.clone(),
            _ => ret,
        };
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
        if this_ok && an.st.funcs[fid].uses_this {
            sum.this_matched += 1;
            if class != Class::Any {
                sum.this_typed += 1;
            }
        }
        let trusted = an.st.funcs[fid].trust != 0 || (this_ok && an.st.funcs[fid].uses_this);
        if class != Class::Any && !trusted {
            sum.sound_typed += 1;
        }
        let mut guard = None;
        if trusted && class != Class::Any && excluded.is_none_or(|r| r == Reason::This) {
            let mut parts = Vec::new();
            let mut ok = true;
            let pnames: Vec<String> = an.st.funcs[fid].params.iter().flatten().cloned().collect();
            for n in pnames {
                let t = an.st.binds[&(fid, n.clone())].ty.clone();
                match an.guard_js(&t, &n, fid, 0) {
                    Some(g) if g == "true" => {}
                    Some(g) => parts.push(g),
                    None => ok = false,
                }
            }
            if this_ok && an.st.funcs[fid].uses_this {
                let t = an.this_ty(fid);
                match an.guard_js(&t, "this", fid, 0) {
                    Some(g) if g == "true" => {}
                    Some(g) => parts.push(g),
                    None => ok = false,
                }
            }
            if ok {
                guard = Some(if parts.is_empty() {
                    "true".to_string()
                } else {
                    parts.join(" && ")
                });
                sum.guarded += 1;
            }
        }
        if an.st.funcs[fid].trust & TRUST_EVIDENCE != 0 {
            sum.ev_matched += 1;
            if class != Class::Any {
                sum.ev_typed += 1;
            }
        }
        if dts_fn {
            sum.dts_matched += 1;
            if class != Class::Any {
                sum.dts_typed += 1;
            } else if let Some(r) = reasons.first() {
                *sum.dts_reason.entry(*r).or_default() += 1;
            }
        }
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
            dts: dts_fn,
            trust: an.st.funcs[fid].trust,
            this_typed: this_ok && an.st.funcs[fid].uses_this,
            guard,
            exports: an.st.funcs[fid]
                .exports
                .iter()
                .filter(|(_, n)| n != "<factory>")
                .map(|(_, n)| n.clone())
                .collect(),
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
    /// (class key, hi of the class name, already-declared property names)
    classes: Vec<((usize, String), u32, Vec<String>)>,
    /// (function, offset of its block body)
    blocks: Vec<(usize, u32, u32)>,
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
        if let Some((lo, hi)) = f.block_span() {
            self.blocks.push((fid, lo, hi));
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
    fn visit_class_decl(&mut self, n: &a::ClassDecl) {
        let declared = n
            .class
            .body
            .iter()
            .filter_map(|m| match m {
                a::ClassMember::ClassProp(p) => match &p.key {
                    a::PropName::Ident(i) => Some(i.sym.to_string()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        self.classes.push((
            (*self.cur.last().unwrap(), n.ident.sym.to_string()),
            n.ident.span.hi.0,
            declared,
        ));
        n.class.visit_with(self);
    }
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
        classes: vec![],
        blocks: vec![],
    };
    module.visit_with(&mut ann);
    let blocks = std::mem::take(&mut ann.blocks);
    let (params, uninit, classes) = (
        std::mem::take(&mut ann.params),
        std::mem::take(&mut ann.uninit),
        std::mem::take(&mut ann.classes),
    );
    let off = |p: u32| cm.lookup_byte_offset(thaw_parser::common::BytePos(p)).pos.0 as usize;
    let mut ins: Vec<(usize, String)> = Vec::new();
    let ty_of = |an: &mut An, fid: usize, n: &str, is_param: bool| -> Option<String> {
        let t = an.st.binds.get(&(fid, n.to_string()))?.ty.clone();
        if t == Ty::Bottom {
            return Some("any".into());
        }
        let (c, why) = an.classify(&t, &mut HashSet::new());
        if c == Class::Any && why == Some(Reason::Escaped) && is_param {
            // unknown only because the function is part of the API: leave the TS pipeline's own
            // call-site inference alone (an explicit `any` would make it strictly worse)
            return None;
        }
        let s = if c == Class::Any {
            "any".into()
        } else {
            an.ts(&t, 0)
        };
        // function values are left to the TS pipeline's own inference
        (!s.contains("Function") && !s.contains("NodeModule")).then_some(s)
    };
    let want_returns = an.cfg.returns;
    // guarded dispatchers for functions whose parameter types rest on a trusted contract
    let mut guarded: HashSet<usize> = HashSet::new();
    let mut plans: Vec<(usize, usize, String)> = Vec::new();
    if an.cfg.guards {
        for (fid, lo, hi) in blocks.iter().copied() {
            let f = &an.st.funcs[fid];
            if f.trust == 0
                || f.async_
                || f.gen
                || f.exotic
                || f.uses_this
                || f.uses_args
                || f.uses_eval
                || f.params.is_empty()
                || !f.params.iter().all(Option::is_some)
            {
                continue;
            }
            let names: Vec<String> = f.params.iter().flatten().cloned().collect();
            let mut guards = Vec::new();
            let mut clone_params = Vec::new();
            let mut ok = true;
            for n in &names {
                let t = an.st.binds[&(fid, n.clone())].ty.clone();
                match an.guard_js(&t, n, fid, 0) {
                    Some(g) if g == "true" => {}
                    Some(g) => guards.push(g),
                    None => {
                        ok = false;
                        break;
                    }
                }
                let (c, _) = an.classify(&t, &mut HashSet::new());
                let s = if c == Class::Any || t == Ty::Bottom {
                    "any".to_string()
                } else {
                    an.ts(&t, 0)
                };
                let s = if s.contains("Function") || s.contains("NodeModule") {
                    "any".to_string()
                } else {
                    s
                };
                clone_params.push(format!("{n}: {s}"));
            }
            if !ok || guards.is_empty() {
                continue;
            }
            let ret = if want_returns {
                let mut ret = an.st.funcs[fid].ret.clone();
                if let (Ty::Any(_), Some(d)) = (&ret, an.dts_ret.get(&fid)) {
                    ret = d.clone();
                }
                let (c, _) = an.classify(&ret, &mut HashSet::new());
                let s = an.ts(&ret, 0);
                if ret == Ty::Bottom
                    || matches!(c, Class::Any | Class::Union | Class::Nullable)
                    || s.contains("Function")
                    || s.contains("NodeModule")
                {
                    String::new()
                } else {
                    format!(": {s}")
                }
            } else {
                String::new()
            };
            guarded.insert(fid);
            plans.push((
                off(lo),
                off(hi),
                format!(
                    " if ({}) {{ return (function ({}){} ",
                    guards.join(" && "),
                    clone_params.join(", "),
                    ret
                ) + "\u{0}BODY\u{0}"
                    + &format!(").call(this, {}); }}", names.join(", ")),
            ));
        }
    }
    // TS needs declared fields for `this.x = ...`: write the inferred instance struct
    for (key, hi, declared) in classes {
        let Some(fields) = an.class_fields(&key) else {
            continue;
        };
        let Some(open) = src[off(hi)..].find('{') else {
            continue;
        };
        let text: String = fields
            .into_iter()
            .filter(|(n, _)| !declared.contains(n))
            .map(|(n, t)| format!("\n  {n}: {t};"))
            .collect();
        if !text.is_empty() {
            ins.push((off(hi) + open + 1, text));
        }
    }
    // return types of functions whose result is a single concrete type (the TS pipeline does not
    // infer through method chains such as `v.scale(2).label()`)
    for (fid, lo, _) in blocks.iter().copied().filter(|_| want_returns) {
        if guarded.contains(&fid) {
            continue;
        }
        let f = &an.st.funcs[fid];
        if f.async_ || f.gen || f.exotic {
            continue;
        }
        let mut ret = f.ret.clone();
        if let (Ty::Any(_), Some(d)) = (&ret, an.dts_ret.get(&fid)) {
            ret = d.clone();
        }
        if ret == Ty::Bottom {
            continue;
        }
        // TS widens a single literal result (`return 0` is `number`), unions of literals stay
        if matches!(&ret, Ty::Set(s) if s.len() == 1) {
            ret = an.widen(&ret);
        }
        let (c, _) = an.classify_pub(&ret);
        if c == Class::Any || c == Class::Union || c == Class::Nullable {
            continue;
        }
        let t = an.ts_pub(&ret);
        if !t.contains("Function") && !t.contains("NodeModule") {
            ins.push((off(lo), format!(": {t} ")));
        }
    }
    for (fid, n, lo, hi) in params {
        let t = if guarded.contains(&fid) {
            Some("any".to_string())
        } else {
            ty_of(&mut an, fid, &n, true)
        };
        let Some(t) = t else {
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
        if let Some(t) = ty_of(&mut an, fid, &n, false) {
            // an initialised declaration keeps the TS pipeline's own inference unless the
            // inferred type is wider than the initializer (a union) or the initializer is open
            if open || (t.contains(" | ") && !t.contains(['[', '{'])) {
                ins.push((off(hi), format!(": {t}")));
            }
        }
    }
    let marks: Vec<(usize, usize)> = ins.iter().map(|(p, s)| (*p, s.len())).collect();
    ins.sort_by_key(|x| std::cmp::Reverse(x.0));
    let mut out = src.to_string();
    for (p, s) in ins {
        out.insert_str(p, &s);
    }
    // phase 2: guarded dispatchers (innermost first); the typed clone copies the edited body
    let start_of = |p: usize| {
        p + marks
            .iter()
            .filter(|(q, _)| *q <= p)
            .map(|(_, l)| l)
            .sum::<usize>()
    };
    let end_of = |p: usize| {
        p + marks
            .iter()
            .filter(|(q, _)| *q < p)
            .map(|(_, l)| l)
            .sum::<usize>()
    };
    plans.sort_by_key(|x| std::cmp::Reverse(x.0));
    let mut applied: Vec<(usize, usize)> = Vec::new();
    for (lo, hi, text) in plans {
        let (s0, e0) = (start_of(lo), end_of(hi));
        let inner: usize = applied
            .iter()
            .filter(|(p, _)| *p >= s0 && *p < e0)
            .map(|(_, l)| l)
            .sum();
        let region = out[s0..e0 + inner].to_string();
        let text = text.replace("\u{0}BODY\u{0}", &region);
        out.insert_str(s0 + 1, &text);
        applied.push((s0 + 1, text.len()));
    }
    Ok(out)
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn json_hist<K: std::fmt::Debug>(m: &BTreeMap<K, usize>) -> String {
    let items: Vec<String> = m
        .iter()
        .map(|(k, v)| format!("{}:{v}", json_str(&format!("{k:?}"))))
        .collect();
    format!("{{{}}}", items.join(","))
}

/// Machine-readable census of one bundle (`thaw inspect`, `inferred.json`).
pub fn summary_json(s: &Summary) -> String {
    let c = |k: Class| s.by_class.get(&k).copied().unwrap_or(0);
    format!(
        "{{\"functions\":{},\"concrete\":{},\"nullable\":{},\"union\":{},\"any\":{},\"dead\":{},\"closed\":{},\
         \"sound_typed\":{},\"guarded\":{},\"dts_typed\":{},\"this_typed\":{},\"ev_typed\":{},\
         \"any_reasons\":{},\"passes\":{}}}",
        s.functions,
        c(Class::Concrete),
        c(Class::Nullable),
        c(Class::Union),
        c(Class::Any),
        c(Class::Dead),
        s.closed,
        s.sound_typed,
        s.guarded,
        s.dts_typed,
        s.this_typed,
        s.ev_typed,
        json_hist(&s.fn_reason),
        s.passes
    )
}

/// Per-function detail: signature, trust sources and entry guard of every typed function.
pub fn report_json(fns: &[FnReport], s: &Summary) -> String {
    let mut items = Vec::new();
    for f in fns
        .iter()
        .filter(|f| !matches!(f.class, Class::Any | Class::Dead))
    {
        let slots: Vec<String> = f
            .slots
            .iter()
            .map(|(n, t, _)| format!("[{},{}]", json_str(n), json_str(t)))
            .collect();
        items.push(format!(
            "{{\"name\":{},\"offset\":{},\"class\":{},\"closed\":{},\"trust\":{},\"this_typed\":{},\"slots\":[{}],\"ret\":{},\"guard\":{}}}",
            json_str(&f.name),
            f.lo,
            json_str(&format!("{:?}", f.class)),
            f.closed,
            f.trust,
            f.this_typed,
            slots.join(","),
            json_str(&f.ret),
            f.guard.as_deref().map_or("null".to_string(), json_str)
        ));
    }
    format!(
        "{{\"summary\":{},\"typed_functions\":[{}]}}",
        summary_json(s),
        items.join(",")
    )
}
