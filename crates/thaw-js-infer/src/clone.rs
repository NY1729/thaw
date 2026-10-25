//! AOT clones: self-contained TypeScript for an exported package function whose signature rests on a
//! `.d.ts` contract, so that Thaw can compile it ahead of time next to the program that calls it.
//!
//! A clone is the function body plus exactly what it needs from the bundle: declared helper
//! functions, literal constants, and constants another module exports (`_index.monthsInYear`
//! becomes its literal). Anything else it touches (objects, regexes, `this`, unknown globals)
//! makes the function ineligible, so a clone never depends on JavaScript runtime state.

use crate::analyze::An;
use crate::prescan::*;
use crate::ty::*;
use crate::Config;
use std::collections::{HashMap, HashSet};
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    Num,
    Str,
    Bool,
}

impl Prim {
    pub fn ts(self) -> &'static str {
        match self {
            Prim::Num => "number",
            Prim::Str => "string",
            Prim::Bool => "boolean",
        }
    }
    /// TypeScript guard on a `Json` value `v`.
    pub fn guard(self, v: &str) -> String {
        format!("typeof {v} === \"{}\"", self.ts())
    }
}

#[derive(Clone, Debug)]
pub struct AotClone {
    /// names the function is exported under (`<default>` for `module.exports = f`)
    pub exports: Vec<String>,
    /// the TypeScript function that implements the clone
    pub fn_name: String,
    pub params: Vec<Prim>,
    pub ret: Prim,
    /// self-contained TypeScript (helpers, constants and `function <fn_name>(...)`)
    pub ts: String,
}

const GLOBALS: &[&str] = &[
    "Math",
    "Number",
    "String",
    "Boolean",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "NaN",
    "Infinity",
    "undefined",
    "Error",
    "TypeError",
    "RangeError",
];

fn prim_of(t: &Ty) -> Option<Prim> {
    if t.is_num() {
        Some(Prim::Num)
    } else if t.is_str() {
        Some(Prim::Str)
    } else if t.only(Atom::Bool) {
        Some(Prim::Bool)
    } else {
        None
    }
}

fn lit_text(e: &a::Expr, src: &str, off: &dyn Fn(u32) -> usize) -> Option<String> {
    use a::Expr as E;
    let span = |lo: u32, hi: u32| src[off(lo)..off(hi)].to_string();
    match e {
        E::Paren(p) => lit_text(&p.expr, src, off),
        E::Lit(a::Lit::Num(n)) => Some(span(n.span.lo.0, n.span.hi.0)),
        E::Lit(a::Lit::Str(s)) => Some(span(s.span.lo.0, s.span.hi.0)),
        E::Lit(a::Lit::Bool(b)) => Some(b.value.to_string()),
        E::Unary(u) if u.op == a::UnaryOp::Minus => lit_text(&u.arg, src, off)
            .filter(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .map(|t| format!("-{t}")),
        _ => None,
    }
}

struct Tables<'a> {
    st: &'a State,
    src: &'a str,
    off: &'a dyn Fn(u32) -> usize,
    cur: Vec<usize>,
    /// `const X = <literal>`
    lits: HashMap<(usize, String), String>,
    /// (module, export name) -> literal values assigned to `exports.NAME`
    export_lits: HashMap<(String, String), Vec<String>>,
    /// (module, export name) assigned something that is not a literal
    export_poison: HashSet<(String, String)>,
}

impl Tables<'_> {
    fn fid(&self) -> usize {
        *self.cur.last().unwrap()
    }
    fn enter(&mut self, key: (u32, u32), f: impl FnOnce(&mut Self)) {
        let Some(&id) = self.st.by_span.get(&key) else {
            return;
        };
        self.cur.push(id);
        f(self);
        self.cur.pop();
    }
}

impl Visit for Tables<'_> {
    fn visit_function(&mut self, n: &a::Function) {
        self.enter((n.span.lo.0, n.span.hi.0), |s| n.visit_children_with(s));
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        self.enter((n.span.lo.0, n.span.hi.0), |s| n.visit_children_with(s));
    }
    fn visit_constructor(&mut self, n: &a::Constructor) {
        self.enter((n.span.lo.0, n.span.hi.0), |s| n.visit_children_with(s));
    }
    fn visit_var_declarator(&mut self, n: &a::VarDeclarator) {
        if let (a::Pat::Ident(i), Some(init)) = (&n.name, &n.init) {
            if let Some(t) = lit_text(init, self.src, self.off) {
                self.lits.insert((self.fid(), i.id.sym.to_string()), t);
            }
        }
        n.visit_children_with(self);
    }
    fn visit_assign_expr(&mut self, n: &a::AssignExpr) {
        if let (a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)), a::AssignOp::Assign) =
            (&n.left, n.op)
        {
            if let (a::Expr::Ident(o), a::MemberProp::Ident(p)) = (&*m.obj, &m.prop) {
                if &*o.sym == "exports" {
                    let key = (self.st.factory_of(self.fid()), p.sym.to_string());
                    // `exports.a = exports.b = void 0` pre-initialisers are not values
                    let mut rhs: &a::Expr = &n.right;
                    while let a::Expr::Paren(p) = rhs {
                        rhs = &p.expr;
                    }
                    let is_void = matches!(rhs, a::Expr::Unary(u) if u.op == a::UnaryOp::Void)
                        || matches!(rhs, a::Expr::Assign(_));
                    if let Some(t) = lit_text(rhs, self.src, self.off) {
                        self.export_lits.entry(key).or_default().push(t);
                    } else if !is_void {
                        self.export_poison.insert(key);
                    }
                }
            }
        }
        n.visit_children_with(self);
    }
}

/// Free references of a function (and of everything nested in it).
#[derive(Default)]
struct FreeRefs {
    /// bare identifier reads (name, function they occur in)
    idents: Vec<(String, usize, u32, u32)>,
    /// `NAME.prop` reads: (lo, hi, name, prop, function)
    members: Vec<(u32, u32, String, String, usize)>,
    bad: bool,
}

struct Free<'a> {
    st: &'a State,
    cur: Vec<usize>,
    out: FreeRefs,
}

impl Visit for Free<'_> {
    fn visit_function(&mut self, n: &a::Function) {
        if let Some(&id) = self.st.by_span.get(&(n.span.lo.0, n.span.hi.0)) {
            self.cur.push(id);
            n.visit_children_with(self);
            self.cur.pop();
        }
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        if let Some(&id) = self.st.by_span.get(&(n.span.lo.0, n.span.hi.0)) {
            self.cur.push(id);
            n.visit_children_with(self);
            self.cur.pop();
        }
    }
    fn visit_this_expr(&mut self, _: &a::ThisExpr) {
        self.out.bad = true;
    }
    fn visit_regex(&mut self, _: &a::Regex) {
        self.out.bad = true;
    }
    fn visit_expr(&mut self, e: &a::Expr) {
        let at = *self.cur.last().unwrap();
        match e {
            a::Expr::Ident(i) => {
                self.out
                    .idents
                    .push((i.sym.to_string(), at, i.span.lo.0, i.span.hi.0))
            }
            a::Expr::Member(m) => {
                if let (a::Expr::Ident(o), a::MemberProp::Ident(p)) = (&*m.obj, &m.prop) {
                    self.out.members.push((
                        m.span.lo.0,
                        m.span.hi.0,
                        o.sym.to_string(),
                        p.sym.to_string(),
                        at,
                    ));
                } else {
                    e.visit_children_with(self)
                }
            }
            a::Expr::Class(_) | a::Expr::Await(_) | a::Expr::Yield(_) | a::Expr::TaggedTpl(_) => {
                self.out.bad = true
            }
            _ => e.visit_children_with(self),
        }
    }
}

/// Where a function's body is and what it references from outside.
struct Located {
    body: (u32, u32),
    expr_body: bool,
    refs: FreeRefs,
}

struct Locate<'a> {
    st: &'a State,
    target: (u32, u32),
    found: Option<Located>,
}

impl Locate<'_> {
    fn run(&self, fid: usize, params: &[&a::Pat], body: &dyn Fn(&mut Free<'_>)) -> FreeRefs {
        let mut free = Free {
            st: self.st,
            cur: vec![fid],
            out: FreeRefs::default(),
        };
        for p in params {
            p.visit_with(&mut free);
        }
        body(&mut free);
        free.out
    }
}

impl Visit for Locate<'_> {
    fn visit_function(&mut self, n: &a::Function) {
        let key = (n.span.lo.0, n.span.hi.0);
        if key == self.target && self.found.is_none() {
            if let (Some(&fid), Some(b)) = (self.st.by_span.get(&key), n.body.as_ref()) {
                let pats: Vec<&a::Pat> = n.params.iter().map(|p| &p.pat).collect();
                let refs = self.run(fid, &pats, &|f| b.visit_with(f));
                self.found = Some(Located {
                    body: (b.span.lo.0, b.span.hi.0),
                    expr_body: false,
                    refs,
                });
            }
            return;
        }
        n.visit_children_with(self);
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        let key = (n.span.lo.0, n.span.hi.0);
        if key == self.target && self.found.is_none() {
            if let Some(&fid) = self.st.by_span.get(&key) {
                let pats: Vec<&a::Pat> = n.params.iter().collect();
                let (body, expr_body) = match &*n.body {
                    a::ArrowFunctionBody::FunctionBody(b) => ((b.span.lo.0, b.span.hi.0), false),
                    a::ArrowFunctionBody::Expr(e) => {
                        use thaw_parser::common::Spanned;
                        ((e.span().lo.0, e.span().hi.0), true)
                    }
                };
                let refs = self.run(fid, &pats, &|f| n.body.visit_with(f));
                self.found = Some(Located {
                    body,
                    expr_body,
                    refs,
                });
            }
            return;
        }
        n.visit_children_with(self);
    }
}

struct Builder<'a> {
    an: &'a An,
    module: &'a a::Module,
    tables: &'a Tables<'a>,
    off: &'a dyn Fn(u32) -> usize,
    /// text edits (absolute offsets) collected while resolving references
    edits: Vec<(usize, usize, String)>,
    consts: Vec<(String, String)>,
    /// helper function ids, in dependency order (callees first)
    helpers: Vec<usize>,
    names: HashSet<String>,
    visiting: HashSet<usize>,
    /// the clone being built: its own name inside the bundle is renamed to this
    target: usize,
    fname: String,
}

impl Builder<'_> {
    fn inside(&self, mut f: usize, root: usize) -> bool {
        loop {
            if f == root {
                return true;
            }
            if f == 0 {
                return false;
            }
            f = self.an.st.funcs[f].parent;
        }
    }

    fn locate(&self, fid: usize) -> Option<Located> {
        let f = &self.an.st.funcs[fid];
        let mut l = Locate {
            st: &self.an.st,
            target: (f.lo, f.hi),
            found: None,
        };
        self.module.visit_with(&mut l);
        l.found
    }

    /// Resolve everything `fid` needs from outside the function. `false` = not self-contained.
    fn resolve_function(&mut self, fid: usize) -> bool {
        if !self.visiting.insert(fid) {
            return false; // recursion through helpers: keep it simple
        }
        let Some(loc) = self.locate(fid) else {
            return false;
        };
        if loc.refs.bad {
            return false;
        }
        let st = &self.an.st;
        for (lo, hi, name, prop, at) in loc.refs.members {
            match st.resolve(at, &name) {
                Some(key) if !self.inside(key.0, fid) => {
                    // `NAME.prop` where NAME = require('./module') and the module exports a literal
                    let Some(target) = st.binds.get(&key).and_then(|b| b.req.clone()) else {
                        return false;
                    };
                    let k = (target, prop);
                    match self.tables.export_lits.get(&k) {
                        Some(v) if v.len() == 1 && !self.tables.export_poison.contains(&k) => {
                            self.edits
                                .push(((self.off)(lo), (self.off)(hi), v[0].clone()));
                        }
                        _ => return false,
                    }
                }
                Some(_) => {}
                None => {
                    if !GLOBALS.contains(&name.as_str()) {
                        return false;
                    }
                }
            }
        }
        for (name, at, ilo, ihi) in loc.refs.idents {
            match st.resolve(at, &name) {
                None => {
                    if !GLOBALS.contains(&name.as_str()) {
                        return false;
                    }
                }
                Some(key)
                    if key.0 == self.target
                        && st.funcs[key.0].declared.get(&name) == Some(&DK::SelfName) =>
                {
                    // a named function expression calling itself
                    self.edits
                        .push(((self.off)(ilo), (self.off)(ihi), self.fname.clone()));
                }
                Some(key) if self.inside(key.0, fid) => {}
                Some(key) => {
                    let Some(b) = st.binds.get(&key) else {
                        return false;
                    };
                    if b.n_assign == 1 && b.fn_value == Some(self.target) {
                        // the function calls itself under its bundle name
                        self.edits
                            .push(((self.off)(ilo), (self.off)(ihi), self.fname.clone()));
                    } else if b.n_assign == 1 && b.fn_value == Some(fid) {
                        // a helper calling itself keeps its own (nested) name
                    } else if let Some(lit) = self.tables.lits.get(&key).filter(|_| b.n_assign == 1)
                    {
                        if self.names.insert(name.clone()) {
                            self.consts.push((name, lit.clone()));
                        }
                    } else if let (1, Some(h)) = (b.n_assign, b.fn_value) {
                        let is_decl = st.funcs[key.0].declared.get(&name) == Some(&DK::Fn);
                        let hf = &st.funcs[h];
                        if !is_decl || hf.uses_this || hf.uses_args || hf.async_ || hf.gen {
                            return false;
                        }
                        if !self.helpers.contains(&h) {
                            if !self.names.insert(name.clone()) || !self.resolve_function(h) {
                                return false;
                            }
                            self.helpers.push(h);
                        }
                    } else {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// `function name(...) {` -> `function name(...): ret {` unless a result type is already there.
fn with_declared_result(ts: &str, fname: &str, ret: Prim) -> String {
    let head = format!("function {fname}(");
    let Some(start) = ts.find(&head) else {
        return ts.to_string();
    };
    let mut depth = 1;
    let mut end = start + head.len();
    for (i, ch) in ts[end..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end += i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if ts[end..].trim_start().starts_with(':') {
        return ts.to_string();
    }
    format!("{}: {}{}", &ts[..end], ret.ts(), &ts[end..])
}

fn text_with_edits(src: &str, lo: usize, hi: usize, edits: &[(usize, usize, String)]) -> String {
    let mut inner: Vec<&(usize, usize, String)> = edits
        .iter()
        .filter(|(l, h, _)| *l >= lo && *h <= hi)
        .collect();
    inner.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = src[lo..hi].to_string();
    for (l, h, r) in inner {
        out.replace_range(l - lo..h - lo, r);
    }
    out
}

/// Clones for every exported function of `src` whose parameters and result are plain
/// numbers / strings / booleans according to the `.d.ts` in `cfg` and that is self-contained.
/// `prefix` namespaces the generated function names.
pub fn extract_clones(src: &str, cfg: Config, prefix: &str) -> Result<Vec<AotClone>, String> {
    let (mut an, _, cm, module) = crate::report::run(src, cfg)?;
    let off = |p: u32| cm.lookup_byte_offset(thaw_parser::common::BytePos(p)).pos.0 as usize;
    // candidates: exported, contract-typed, primitive signature
    struct Cand {
        fid: usize,
        exports: Vec<String>,
        params: Vec<(String, Prim)>,
        ret: Prim,
    }
    let mut cands = Vec::new();
    for fid in 1..an.st.funcs.len() {
        let f = &an.st.funcs[fid];
        let mut exports: Vec<String> = f
            .exports
            .iter()
            .filter(|(_, n)| n != "<factory>")
            .map(|(_, n)| n.clone())
            .collect();
        // `module.exports = clamp` is `import clamp from ...`: the declaration knows it as `clamp`
        if let Some(n) = &f.prop_name {
            if !exports.is_empty() && !exports.contains(n) {
                exports.push(n.clone());
            }
        }
        if exports.is_empty()
            || f.trust & TRUST_DTS == 0
            || f.async_
            || f.gen
            || f.exotic
            || f.uses_this
            || f.uses_args
            || f.uses_eval
            || f.params.is_empty()
            || f.params.len() > 8
            || !f.params.iter().all(Option::is_some)
            || f.defaults.iter().any(|d| *d)
        {
            continue;
        }
        let names: Vec<String> = f.params.iter().flatten().cloned().collect();
        let mut params = Vec::new();
        for n in &names {
            let Some(b) = an.st.binds.get(&(fid, n.clone())) else {
                break;
            };
            match prim_of(&b.ty) {
                Some(p) => params.push((n.clone(), p)),
                None => break,
            }
        }
        if params.len() != names.len() {
            continue;
        }
        // the contract's result type wins: the inferred one can be wider than the truth
        let mut ret = f.ret.clone();
        if let Some(d) = an.dts_ret.get(&fid).cloned() {
            if prim_of(&an.widen(&d)).is_some() {
                ret = d;
            }
        }
        let ret = an.widen(&ret);
        let Some(ret) = prim_of(&ret) else { continue };
        cands.push(Cand {
            fid,
            exports,
            params,
            ret,
        });
    }
    let mut tables = Tables {
        st: &an.st,
        src,
        off: &off,
        cur: vec![0],
        lits: HashMap::new(),
        export_lits: HashMap::new(),
        export_poison: HashSet::new(),
    };
    module.visit_with(&mut tables);
    let (lits, export_lits, export_poison) = (
        std::mem::take(&mut tables.lits),
        std::mem::take(&mut tables.export_lits),
        std::mem::take(&mut tables.export_poison),
    );
    drop(tables);
    let tables = Tables {
        st: &an.st,
        src,
        off: &off,
        cur: vec![0],
        lits,
        export_lits,
        export_poison,
    };
    let mut out = Vec::new();
    for (idx, c) in cands.into_iter().enumerate() {
        let fname = format!(
            "{prefix}{idx}_{}",
            c.exports[0].replace(|ch: char| !ch.is_alphanumeric(), "_")
        );
        let mut b = Builder {
            an: &an,
            module: &module,
            tables: &tables,
            off: &off,
            edits: vec![],
            consts: vec![],
            helpers: vec![],
            names: HashSet::new(),
            visiting: HashSet::new(),
            target: c.fid,
            fname: fname.clone(),
        };
        if !b.resolve_function(c.fid) {
            continue;
        }
        let Some(loc) = b.locate(c.fid) else { continue };
        let fname = format!(
            "{prefix}{idx}_{}",
            c.exports[0].replace(|ch: char| !ch.is_alphanumeric(), "_")
        );
        if b.names.contains(&fname) {
            continue;
        }
        let mut prelude = String::new();
        for (n, lit) in &b.consts {
            prelude.push_str(&format!("var {n} = {lit};\n"));
        }
        for h in &b.helpers {
            let hf = &an.st.funcs[*h];
            prelude.push_str(&text_with_edits(src, off(hf.lo), off(hf.hi), &b.edits));
            prelude.push('\n');
        }
        let (blo, bhi) = (off(loc.body.0), off(loc.body.1));
        let body = text_with_edits(src, blo, bhi, &b.edits);
        // helpers and constants live inside the clone: no names leak into the program
        let body = if loc.expr_body {
            format!("{{ {prelude} return {body}; }}")
        } else {
            format!("{{ {prelude} {}", &body[1..])
        };
        let mut mini = String::new();
        let plist: Vec<&str> = c.params.iter().map(|(n, _)| n.as_str()).collect();
        mini.push_str(&format!(
            "function {fname}({}) {body}\n__thaw_export({fname});\n",
            plist.join(", ")
        ));
        let dts = format!(
            "declare function {fname}({}): {};",
            c.params
                .iter()
                .map(|(n, p)| format!("{n}: {}", p.ts()))
                .collect::<Vec<_>>()
                .join(", "),
            c.ret.ts()
        );
        let mcfg = Config {
            dts: Some(std::sync::Arc::new(crate::dts::DtsIndex::parse(&dts)?)),
            returns: true,
            ..Default::default()
        };
        let Ok(ts) = crate::annotate(&mini, mcfg) else {
            continue;
        };
        let ts: String = ts
            .lines()
            .filter(|l| !l.contains("__thaw_export("))
            .collect::<Vec<_>>()
            .join("\n");
        let ts = with_declared_result(&ts, &fname, c.ret);
        out.push(AotClone {
            exports: c.exports,
            fn_name: fname,
            params: c.params.iter().map(|(_, p)| *p).collect(),
            ret: c.ret,
            ts,
        });
    }
    Ok(out)
}
