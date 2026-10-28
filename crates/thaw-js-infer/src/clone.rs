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

macro_rules! why {
    ($($t:tt)*) => {
        if std::env::var("THAW_CLONE_DEBUG").is_ok() {
            eprintln!("CLONEDBG {}", format!($($t)*));
        }
    };
}

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

/// Parameter type of a clone: a primitive, an array of primitives, or a flat record of
/// primitives / primitive arrays (the shapes the typed-call guard can check and rebuild).
#[derive(Clone, Debug, PartialEq)]
pub enum CTy {
    Prim(Prim),
    Arr(Prim),
    /// fields sorted by name
    Obj(Vec<(String, CTy)>),
}

impl From<Prim> for CTy {
    fn from(p: Prim) -> CTy {
        CTy::Prim(p)
    }
}

impl PartialEq<Prim> for CTy {
    fn eq(&self, other: &Prim) -> bool {
        *self == CTy::Prim(*other)
    }
}

impl CTy {
    pub fn ts(&self) -> String {
        match self {
            CTy::Prim(p) => p.ts().to_string(),
            CTy::Arr(p) => format!("{}[]", p.ts()),
            CTy::Obj(f) => format!(
                "{{ {} }}",
                f.iter()
                    .map(|(k, t)| format!("{k}: {}", t.ts()))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        }
    }

    /// The declared type this clone parameter corresponds to.
    pub fn to_dt(&self) -> crate::dts::DT {
        use crate::dts::DT;
        let prim = |p: &Prim| match p {
            Prim::Num => DT::Num,
            Prim::Str => DT::Str,
            Prim::Bool => DT::Bool,
        };
        match self {
            CTy::Prim(p) => prim(p),
            CTy::Arr(p) => DT::Arr(Box::new(prim(p))),
            CTy::Obj(f) => DT::Obj(f.iter().map(|(k, t)| (k.clone(), t.to_dt())).collect()),
        }
    }

    /// A well-typed expression of this type (to probe a clone with).
    pub fn dummy(&self) -> String {
        match self {
            CTy::Prim(Prim::Num) => "0".into(),
            CTy::Prim(Prim::Str) => "\"\"".into(),
            CTy::Prim(Prim::Bool) => "false".into(),
            CTy::Arr(_) => "[]".into(),
            CTy::Obj(f) => format!(
                "{{ {} }}",
                f.iter()
                    .map(|(k, t)| format!("{k}: {}", t.dummy()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// Guard on a `Json` value `v` (primitives only; containers use [`AotClone::helpers`]).
    pub fn guard(&self, v: &str) -> Option<String> {
        match self {
            CTy::Prim(p) => Some(p.guard(v)),
            _ => None,
        }
    }

    /// Parse the TypeScript spelling of a declared type (`number`, `string[]`, `{ a: number; b: string[] }`).
    pub fn parse(text: &str) -> Option<CTy> {
        let t = text.trim();
        let prim = |s: &str| match s.trim() {
            "number" => Some(Prim::Num),
            "string" => Some(Prim::Str),
            "boolean" => Some(Prim::Bool),
            _ => None,
        };
        if let Some(inner) = t.strip_suffix("[]") {
            return prim(inner).map(CTy::Arr);
        }
        if let Some(body) = t.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            let mut fields = Vec::new();
            for part in split_top(body, &[';', ',']) {
                if part.trim().is_empty() {
                    continue;
                }
                let (k, v) = part.split_once(':')?;
                let k = k.trim();
                if k.is_empty()
                    || k.ends_with('?')
                    || !k
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                {
                    return None;
                }
                fields.push((k.to_string(), CTy::parse(v)?));
            }
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            return (!fields.is_empty()).then_some(CTy::Obj(fields));
        }
        prim(t).map(CTy::Prim)
    }

    /// `function N_ok(v: Json): boolean` / `function N_cv(v: Json): <ts>` over a `Json` value.
    fn helpers(&self, name: &str, out: &mut String) {
        match self {
            CTy::Prim(p) => {
                let t = p.ts();
                let zero = match p {
                    Prim::Num => "0",
                    Prim::Str => "\"\"",
                    Prim::Bool => "false",
                };
                out.push_str(&format!(
                    "function {name}_ok(v: Json): boolean {{ return typeof v === \"{t}\"; }}\nfunction {name}_cv(v: Json): {t} {{ if (typeof v === \"{t}\") {{ return v; }} return {zero}; }}\n"
                ));
            }
            CTy::Arr(p) => {
                CTy::Prim(*p).helpers(&format!("{name}_e"), out);
                out.push_str(&format!(
                    "function {name}_ok(v: Json): boolean {{ if (!Array.isArray(v)) {{ return false; }} for (let i = 0; i < v.length; i++) {{ if (!{name}_e_ok(v[i])) {{ return false; }} }} return true; }}\nfunction {name}_cv(v: Json): {}[] {{ const r: {}[] = []; if (Array.isArray(v)) {{ for (let i = 0; i < v.length; i++) {{ r.push({name}_e_cv(v[i])); }} }} return r; }}\n",
                    p.ts(),
                    p.ts()
                ));
            }
            CTy::Obj(fields) => {
                for (i, (_, t)) in fields.iter().enumerate() {
                    t.helpers(&format!("{name}_f{i}"), out);
                }
                let oks = fields
                    .iter()
                    .enumerate()
                    .map(|(i, (k, _))| format!(" && {name}_f{i}_ok(v.{k})"))
                    .collect::<String>();
                let cvs = fields
                    .iter()
                    .enumerate()
                    .map(|(i, (k, _))| format!("{k}: {name}_f{i}_cv(v.{k})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "function {name}_ok(v: Json): boolean {{ return typeof v === \"object\" && v !== null && !Array.isArray(v){oks}; }}\nfunction {name}_cv(v: Json): {} {{ return {{ {cvs} }}; }}\n",
                    self.ts()
                ));
            }
        }
    }
}

/// Split at top-level (outside braces / brackets) occurrences of any of `seps`.
fn split_top<'a>(s: &'a str, seps: &[char]) -> Vec<&'a str> {
    let (mut depth, mut start, mut out) = (0i32, 0, Vec::new());
    for (i, ch) in s.char_indices() {
        match ch {
            '{' | '[' | '(' | '<' => depth += 1,
            '}' | ']' | ')' | '>' => depth -= 1,
            c if depth == 0 && seps.contains(&c) => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

#[derive(Clone, Debug)]
pub struct AotClone {
    /// names the function is exported under (`<default>` for `module.exports = f`)
    pub exports: Vec<String>,
    /// the TypeScript function that implements the clone
    pub fn_name: String,
    pub params: Vec<CTy>,
    /// a primitive or an array of primitives (fresh: nothing aliased from the caller)
    pub ret: CTy,
    /// self-contained TypeScript (helpers, constants and `function <fn_name>(...)`)
    pub ts: String,
}

impl AotClone {
    /// Conversion helpers for the container parameters (`<fn>_p<i>_ok` / `_cv` over a `Json` value);
    /// the primitive ones are checked inline by the dispatcher.
    pub fn helpers(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.params.iter().enumerate() {
            if !matches!(p, CTy::Prim(_)) {
                p.helpers(&format!("{}_p{i}", self.fn_name), &mut out);
            }
        }
        out
    }
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

/// The clone parameter type for an inferred type: a primitive, an array of primitives or a flat
/// record (no class instance, no optional field) of those.
fn cty_of(an: &mut An, t: &Ty) -> Option<CTy> {
    if let Some(p) = prim_of(t) {
        return Some(CTy::Prim(p));
    }
    let Ty::Set(atoms) = t else { return None };
    let [at] = atoms.as_slice() else { return None };
    match *at {
        Atom::Arr(s) => {
            let s = an.find(s);
            match an.sites[s].clone() {
                Site::Arr(e) => Some(CTy::Arr(prim_of(&e)?)),
                _ => None,
            }
        }
        Atom::Obj(s) => {
            let s = an.find(s);
            if an.site_class_of(s).is_some() {
                return None;
            }
            let Site::Obj(m) = an.sites[s].clone() else {
                return None;
            };
            if m.is_empty() || m.len() > 8 {
                return None;
            }
            let mut fields = Vec::new();
            for (k, v) in m {
                let ident = k
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    && !k.starts_with(|c: char| c.is_ascii_digit());
                match cty_of(an, &v)? {
                    CTy::Obj(_) => return None,
                    f if ident => fields.push((k, f)),
                    _ => return None,
                }
            }
            Some(CTy::Obj(fields))
        }
        _ => None,
    }
}

/// A clone result type: a primitive or an array of primitives.
fn ret_cty(an: &mut An, t: &Ty) -> Option<CTy> {
    match cty_of(an, t)? {
        r @ (CTy::Prim(_) | CTy::Arr(_)) => Some(r),
        CTy::Obj(_) => None,
    }
}

/// The clone type of a declared type, straight from the declaration (no flow involved).
fn cty_of_dt(t: &crate::dts::DT, top: bool) -> Option<CTy> {
    use crate::dts::DT;
    let prim = |t: &DT| match t {
        DT::Num => Some(Prim::Num),
        DT::Str => Some(Prim::Str),
        DT::Bool => Some(Prim::Bool),
        _ => None,
    };
    match t {
        DT::Arr(e) => prim(e).map(CTy::Arr),
        DT::Obj(fields) if top && !fields.is_empty() && fields.len() <= 8 => {
            let mut out = Vec::new();
            for (k, v) in fields {
                let ident = k
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                    && !k.starts_with(|c: char| c.is_ascii_digit());
                match cty_of_dt(v, false)? {
                    f if ident => out.push((k.clone(), f)),
                    _ => return None,
                }
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Some(CTy::Obj(out))
        }
        other => prim(other).map(CTy::Prim),
    }
}

/// Declared type of parameter `i` when every overload agrees on it and has `arity` parameters.
fn declared_param(an: &An, fid: usize, i: usize, arity: usize) -> Option<CTy> {
    let sigs = an.dts_fns.get(&fid)?;
    let mut found: Option<CTy> = None;
    for sig in sigs {
        if sig.params.len() != arity {
            return None;
        }
        let t = cty_of_dt(sig.params.get(i)?, true)?;
        match &found {
            Some(prev) if *prev != t => return None,
            _ => found = Some(t),
        }
    }
    found
}

/// Methods that cannot change the receiver (or hand out an alias of it).
const PURE_METHODS: &[&str] = &[
    "join",
    "indexOf",
    "lastIndexOf",
    "includes",
    "slice",
    "concat",
    "map",
    "filter",
    "forEach",
    "reduce",
    "some",
    "every",
    "findIndex",
    "toString",
];

fn member_root(e: &a::Expr) -> Option<&str> {
    match e {
        a::Expr::Ident(i) => Some(&i.sym),
        a::Expr::Member(m) => member_root(&m.obj),
        a::Expr::Paren(p) => member_root(&p.expr),
        _ => None,
    }
}

/// A container parameter is only read: never assigned through, mutated, aliased or passed on.
struct ReadOnly<'a> {
    names: &'a HashSet<String>,
    /// field names that hold arrays (reading one as a value would hand out an alias)
    array_fields: &'a HashSet<String>,
    ok: bool,
    /// `callee(.., p, ..)`: (callee name, argument index) - the callee must only read it too
    passes: Vec<(String, usize)>,
    /// `(array, index)` pairs that are in bounds in the loop body being visited
    safe: Vec<(String, String)>,
    /// arrays known to be non-empty from here on (`if (p.length === 0) throw ...` seen)
    nonempty: HashSet<String>,
    /// `var n = p.length`: n -> p
    lens: HashMap<String, String>,
    /// assignments / updates per identifier, and the loops that account for them
    assigns: HashMap<String, u32>,
    loops: HashMap<String, u32>,
    lens_used: HashSet<String>,
    /// computed member reads seen, and how many of them are proven in bounds
    total: u32,
    proven: u32,
}

fn strip_paren(e: &a::Expr) -> &a::Expr {
    match e {
        a::Expr::Paren(p) => strip_paren(&p.expr),
        other => other,
    }
}

fn is_num(e: &a::Expr, v: f64) -> bool {
    matches!(strip_paren(e), a::Expr::Lit(a::Lit::Num(n)) if n.value == v)
}

/// `P.length` for a plain identifier `P`.
fn length_of(e: &a::Expr) -> Option<&str> {
    match strip_paren(e) {
        a::Expr::Member(m) if matches!(&m.prop, a::MemberProp::Ident(p) if &*p.sym == "length") => {
            match strip_paren(&m.obj) {
                a::Expr::Ident(i) => Some(&i.sym),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `if (P.length === 0) throw/return ...` (or `!P.length`): afterwards P is non-empty.
fn empty_guard(s: &a::Stmt) -> Option<String> {
    let a::Stmt::If(i) = s else { return None };
    if i.alt.is_some() {
        return None;
    }
    let exits = |s: &a::Stmt| matches!(s, a::Stmt::Throw(_) | a::Stmt::Return(_));
    let throws = match &*i.cons {
        a::Stmt::Block(b) => b.stmts.last().is_some_and(exits),
        other => exits(other),
    };
    if !throws {
        return None;
    }
    match strip_paren(&i.test) {
        a::Expr::Bin(b) if matches!(b.op, a::BinaryOp::EqEqEq | a::BinaryOp::EqEq) => {
            if is_num(&b.right, 0.0) {
                length_of(&b.left).map(str::to_string)
            } else if is_num(&b.left, 0.0) {
                length_of(&b.right).map(str::to_string)
            } else {
                None
            }
        }
        a::Expr::Unary(u) if u.op == a::UnaryOp::Bang => length_of(&u.arg).map(str::to_string),
        _ => None,
    }
}

impl ReadOnly<'_> {
    fn rooted(&self, e: &a::Expr) -> bool {
        member_root(e).is_some_and(|r| self.names.contains(r))
    }
    fn target_rooted(&self, t: &a::AssignTarget) -> bool {
        match t {
            a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)) => {
                self.rooted(&a::Expr::Member(m.clone()))
            }
            a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(i)) => {
                self.names.contains(&*i.id.sym)
            }
            _ => false,
        }
    }
}

impl ReadOnly<'_> {
    fn stmts(&mut self, list: &[a::Stmt]) {
        for s in list {
            s.visit_with(self);
            if let Some(p) = empty_guard(s) {
                self.nonempty.insert(p);
            }
        }
    }

    /// `p[idx]` is a read of element `idx` of `p` that cannot be out of bounds.
    fn index_safe(&mut self, p: &str, idx: &a::Expr) -> bool {
        match strip_paren(idx) {
            a::Expr::Ident(i) => {
                if self.safe.iter().any(|(a, v)| a == p && v == &*i.sym) {
                    return true;
                }
                false
            }
            e if is_num(e, 0.0) => self.nonempty.contains(p),
            a::Expr::Bin(b) if b.op == a::BinaryOp::Sub && is_num(&b.right, 1.0) => {
                length_of(&b.left) == Some(p) && self.nonempty.contains(p)
            }
            _ => false,
        }
    }

    /// `for (var i = K; i < p.length; i++)` (the bound may be a `var n = p.length` alias):
    /// the (array, index) pair, with the loop-variable and alias bookkeeping recorded.
    fn loop_pattern(&mut self, n: &a::ForStmt) -> Option<(String, String)> {
        let a::VarDeclOrExpr::VarDecl(decl) = n.init.as_ref()? else {
            return None;
        };
        let mut idx = None;
        for d in &decl.decls {
            let a::Pat::Ident(b) = &d.name else {
                return None;
            };
            let init = d.init.as_ref()?;
            if let Some(p) = length_of(init).filter(|p| self.names.contains(*p)) {
                self.lens.insert(b.id.sym.to_string(), p.to_string());
            } else if matches!(strip_paren(init), a::Expr::Lit(a::Lit::Num(k)) if k.value >= 0.0 && k.value.fract() == 0.0)
                && idx.is_none()
            {
                idx = Some(b.id.sym.to_string());
            } else {
                return None;
            }
        }
        let i = idx?;
        let a::Expr::Bin(t) = strip_paren(n.test.as_deref()?) else {
            return None;
        };
        if t.op != a::BinaryOp::Lt
            || !matches!(strip_paren(&t.left), a::Expr::Ident(l) if *l.sym == *i)
        {
            return None;
        }
        let p = match strip_paren(&t.right) {
            a::Expr::Ident(v) => {
                let p = self.lens.get(&*v.sym)?.clone();
                self.lens_used.insert(v.sym.to_string());
                p
            }
            other => length_of(other)
                .filter(|p| self.names.contains(*p))?
                .to_string(),
        };
        match strip_paren(n.update.as_deref()?) {
            a::Expr::Update(u)
                if u.op == a::UpdateOp::PlusPlus
                    && matches!(strip_paren(&u.arg), a::Expr::Ident(l) if *l.sym == *i) => {}
            _ => return None,
        }
        Some((p, i))
    }

    /// Loop variables and length aliases must not be assigned anywhere but their loop header.
    fn bookkeeping_ok(&self) -> bool {
        self.loops
            .iter()
            .all(|(v, n)| self.assigns.get(v).copied().unwrap_or(0) == *n)
            && self
                .lens_used
                .iter()
                .all(|v| self.assigns.get(v).copied().unwrap_or(0) == 0)
    }
}

impl Visit for ReadOnly<'_> {
    fn visit_for_stmt(&mut self, n: &a::ForStmt) {
        match self.loop_pattern(n) {
            Some((p, i)) => {
                *self.loops.entry(i.clone()).or_default() += 1;
                self.safe.push((p, i));
                n.init.visit_with(self);
                n.test.visit_with(self);
                n.update.visit_with(self);
                n.body.visit_with(self);
                self.safe.pop();
            }
            None => n.visit_children_with(self),
        }
    }
    fn visit_block_stmt(&mut self, n: &a::BlockStmt) {
        self.stmts(&n.stmts);
    }
    fn visit_function_body(&mut self, n: &a::FunctionBody) {
        self.stmts(&n.stmts);
    }
    fn visit_var_declarator(&mut self, n: &a::VarDeclarator) {
        if let (a::Pat::Ident(b), Some(init)) = (&n.name, &n.init) {
            if let Some(p) = length_of(init).filter(|p| self.names.contains(*p)) {
                self.lens.insert(b.id.sym.to_string(), p.to_string());
            }
        }
        n.visit_children_with(self);
    }
    fn visit_assign_expr(&mut self, n: &a::AssignExpr) {
        if let a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(i)) = &n.left {
            *self.assigns.entry(i.id.sym.to_string()).or_default() += 1;
        }
        if self.target_rooted(&n.left) {
            self.ok = false;
        }
        n.right.visit_with(self);
        if let a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)) = &n.left {
            if let a::MemberProp::Computed(c) = &m.prop {
                c.visit_with(self);
            }
        }
    }
    fn visit_update_expr(&mut self, n: &a::UpdateExpr) {
        if self.rooted(&n.arg) {
            self.ok = false;
        }
        if let a::Expr::Ident(i) = strip_paren(&n.arg) {
            *self.assigns.entry(i.sym.to_string()).or_default() += 1;
        }
    }
    fn visit_unary_expr(&mut self, n: &a::UnaryExpr) {
        if n.op == a::UnaryOp::Delete && self.rooted(&n.arg) {
            self.ok = false;
        }
        n.visit_children_with(self);
    }
    fn visit_call_expr(&mut self, n: &a::CallExpr) {
        if let a::Callee::Expr(c) = &n.callee {
            if let a::Expr::Ident(f) = &**c {
                if self.names.contains(&*f.sym) {
                    self.ok = false;
                }
                for (i, arg) in n.args.iter().enumerate() {
                    match (&*arg.expr, arg.spread) {
                        (a::Expr::Ident(x), None) if self.names.contains(&*x.sym) => {
                            self.passes.push((f.sym.to_string(), i))
                        }
                        _ => arg.visit_with(self),
                    }
                }
                return;
            }
            if let a::Expr::Member(m) = &**c {
                if self.rooted(&m.obj) {
                    match &m.prop {
                        a::MemberProp::Ident(p) if PURE_METHODS.contains(&&*p.sym) => {}
                        _ => self.ok = false,
                    }
                    for arg in &n.args {
                        arg.visit_with(self);
                    }
                    return;
                }
            }
        }
        n.visit_children_with(self);
    }
    fn visit_for_of_stmt(&mut self, n: &a::ForOfStmt) {
        if self.rooted(&n.right) {
            n.left.visit_with(self);
            n.body.visit_with(self);
            return;
        }
        n.visit_children_with(self);
    }
    fn visit_expr(&mut self, e: &a::Expr) {
        match e {
            a::Expr::Ident(i) if self.names.contains(&*i.sym) => self.ok = false,
            a::Expr::Member(m) if self.rooted(e) => {
                // terminal read of `p.f` / `p[i]`: an array-valued field would be an alias
                if let a::MemberProp::Ident(p) = &m.prop {
                    if self.array_fields.contains(&*p.sym) {
                        self.ok = false;
                    }
                }
                let mut cur = m;
                loop {
                    if let a::MemberProp::Computed(c) = &cur.prop {
                        self.total += 1;
                        match strip_paren(&cur.obj) {
                            a::Expr::Ident(o) if self.names.contains(&*o.sym) => {
                                let name = o.sym.to_string();
                                if self.index_safe(&name, &c.expr) {
                                    self.proven += 1;
                                } else {
                                    self.ok = false;
                                }
                            }
                            _ => self.ok = false,
                        }
                        c.visit_with(self);
                    }
                    match &*cur.obj {
                        a::Expr::Member(inner) => cur = inner,
                        _ => break,
                    }
                }
            }
            a::Expr::Member(m) => {
                if matches!(m.prop, a::MemberProp::Computed(_)) {
                    self.total += 1;
                }
                e.visit_children_with(self)
            }
            _ => e.visit_children_with(self),
        }
    }
}

/// What [`ReadOnly`] found in one function.
struct RoOutcome {
    ok: bool,
    passes: Vec<(String, usize)>,
    total: u32,
    proven: u32,
}

/// Runs [`ReadOnly`] over the body of the function spanning `target`.
struct FindReadOnly<'a> {
    target: (u32, u32),
    names: &'a HashSet<String>,
    array_fields: &'a HashSet<String>,
    res: Option<RoOutcome>,
}

impl FindReadOnly<'_> {
    fn check(&mut self, f: &dyn Fn(&mut ReadOnly<'_>)) {
        let mut ro = ReadOnly {
            names: self.names,
            array_fields: self.array_fields,
            ok: true,
            passes: vec![],
            safe: vec![],
            nonempty: HashSet::new(),
            lens: HashMap::new(),
            assigns: HashMap::new(),
            loops: HashMap::new(),
            lens_used: HashSet::new(),
            total: 0,
            proven: 0,
        };
        f(&mut ro);
        let ok = ro.ok && ro.bookkeeping_ok();
        self.res = Some(RoOutcome {
            ok,
            passes: ro.passes,
            total: ro.total,
            proven: ro.proven,
        });
    }
}

impl Visit for FindReadOnly<'_> {
    fn visit_function(&mut self, n: &a::Function) {
        if (n.span.lo.0, n.span.hi.0) == self.target && self.res.is_none() {
            if let Some(b) = &n.body {
                self.check(&|ro| b.visit_with(ro));
            }
            return;
        }
        n.visit_children_with(self);
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        if (n.span.lo.0, n.span.hi.0) == self.target && self.res.is_none() {
            self.check(&|ro| n.body.visit_with(ro));
            return;
        }
        n.visit_children_with(self);
    }
}

/// `names` (container parameters of the function `fid`) are only read there, and in every function
/// they are passed to (one level of helpers per call, at most `depth` deep).
fn readonly_ok(
    an: &An,
    module: &a::Module,
    fid: usize,
    names: &HashSet<String>,
    array_fields: &HashSet<String>,
    depth: usize,
    visited: &mut HashSet<usize>,
) -> Option<(u32, u32)> {
    visited.insert(fid);
    let f = &an.st.funcs[fid];
    let mut find = FindReadOnly {
        target: (f.lo, f.hi),
        names,
        array_fields,
        res: None,
    };
    module.visit_with(&mut find);
    let RoOutcome {
        ok,
        passes,
        mut total,
        mut proven,
    } = find.res?;
    if !ok || (!passes.is_empty() && depth == 0) {
        return None;
    }
    for (callee, idx) in passes {
        let key = an.st.resolve(fid, &callee)?;
        let b = an.st.binds.get(&key)?;
        let (1, Some(h)) = (b.n_assign, b.fn_value) else {
            return None;
        };
        let pname = an.st.funcs[h].params.get(idx)?.clone()?;
        if h == fid {
            return None;
        }
        let (t, p) = readonly_ok(
            an,
            module,
            h,
            &HashSet::from([pname]),
            array_fields,
            depth - 1,
            visited,
        )?;
        total += t;
        proven += p;
    }
    Some((total, proven))
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
    /// top-level names of the program the clone is compiled into: a helper with such a name is renamed
    reserved: &'a HashSet<String>,
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

    /// Nested helpers whose name is also a top-level name of the program get a name of their own
    /// (otherwise a call inside the clone can resolve to the top-level function).
    fn helper_name(&self, h: usize, name: &str) -> String {
        if self.reserved.contains(name) {
            format!("{}_h{h}", self.fname)
        } else {
            name.to_string()
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
                        // a helper calling itself (helpers get unique names, see below)
                        self.edits.push((
                            (self.off)(ilo),
                            (self.off)(ihi),
                            self.helper_name(fid, &name),
                        ));
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
                        // nested helpers get names of their own: a helper called `sum` inside the clone
                        // would otherwise be confused with a top-level `sum` of the program
                        self.edits.push((
                            (self.off)(ilo),
                            (self.off)(ihi),
                            self.helper_name(h, &name),
                        ));
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
fn with_declared_result(ts: &str, fname: &str, ret: &str) -> String {
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
    format!("{}: {}{}", &ts[..end], ret, &ts[end..])
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
    let cfg_reserved: HashSet<String> = cfg
        .reserved_names
        .iter()
        .cloned()
        .chain(cfg.dts.iter().flat_map(|d| {
            d.sigs
                .keys()
                .map(|k| k.rsplit('.').next().unwrap_or(k).to_string())
        }))
        .collect();
    let (mut an, _, cm, module) = crate::report::run(src, cfg)?;
    let off = |p: u32| cm.lookup_byte_offset(thaw_parser::common::BytePos(p)).pos.0 as usize;
    // candidates: exported, contract-typed, primitive signature
    struct Cand {
        fid: usize,
        exports: Vec<String>,
        params: Vec<(String, CTy)>,
        ret: CTy,
        in_bounds: bool,
        /// functions whose element reads were checked
        visited: HashSet<usize>,
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
        let declared = match f.prop_name.as_deref() {
            // `module.exports = (a) => ..`: the declaration names the default value
            Some("exports") | None => an.cfg.dts.as_ref().and_then(|d| d.default_name.clone()),
            Some(n) => Some(n.to_string()),
        };
        if let Some(n) = &declared {
            if !exports.is_empty() && !exports.contains(n) {
                exports.push(n.clone());
            }
        }
        if !exports.is_empty() && f.trust & TRUST_DTS == 0 {
            why!("no-dts-contract {:?} name={:?}", exports, f.prop_name);
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
        let mut in_bounds = false;
        let mut visited = HashSet::new();
        let names: Vec<String> = f.params.iter().flatten().cloned().collect();
        let fret = f.ret.clone();
        let mut params = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let Some(flowed) = an.st.binds.get(&(fid, n.clone())).map(|b| b.ty.clone()) else {
                break;
            };
            // the clone runs only for external callers, which the contract describes: use the
            // declared type, not the flow-joined one (internal calls with imprecise arguments widen it)
            let ty = match declared_param(&an, fid, i, names.len()) {
                Some(t) => Some(t),
                None => cty_of(&mut an, &flowed),
            };
            match ty {
                Some(p) => params.push((n.clone(), p)),
                None => {
                    why!("param-ty {}", an.ts_pub(&flowed));
                    break;
                }
            }
        }
        if params.len() != names.len() {
            why!("param-type {:?}", exports);
            continue;
        }
        // container parameters are copied at the call boundary, so the clone must only read them
        let containers: HashSet<String> = params
            .iter()
            .filter(|(_, t)| !matches!(t, CTy::Prim(_)))
            .map(|(n, _)| n.clone())
            .collect();
        if !containers.is_empty() {
            let array_fields: HashSet<String> = params
                .iter()
                .filter_map(|(_, t)| match t {
                    CTy::Obj(f) => Some(f),
                    _ => None,
                })
                .flatten()
                .filter(|(_, t)| matches!(t, CTy::Arr(_)))
                .map(|(k, _)| k.clone())
                .collect();
            match readonly_ok(
                &an,
                &module,
                fid,
                &containers,
                &array_fields,
                3,
                &mut visited,
            ) {
                // every element read in the clone must be provably in bounds: the TS lowering
                // does not reproduce `undefined` for an out-of-range index
                Some((total, proven)) if total == proven => in_bounds = true,
                other => {
                    why!("mutated-or-unbounded-container {:?} {:?}", exports, other);
                    continue;
                }
            }
        }
        // the contract's result type wins: the inferred one can be wider than the truth
        let mut ret = fret;
        if let Some(d) = an.dts_ret.get(&fid).cloned() {
            let w = an.widen(&d);
            if ret_cty(&mut an, &w).is_some() {
                ret = d;
            }
        }
        let ret = an.widen(&ret);
        let Some(ret) = ret_cty(&mut an, &ret) else {
            why!("ret-type {:?} {}", exports, an.ts_pub(&ret));
            continue;
        };
        if matches!(ret, CTy::Arr(_)) && containers.is_empty() {
            // a result array is built here: every element read in the body must be provably in bounds
            let none = HashSet::new();
            match readonly_ok(&an, &module, fid, &none, &none, 3, &mut visited) {
                Some((total, proven)) if total == proven => in_bounds = true,
                other => {
                    why!("unbounded-result-array {:?} {:?}", exports, other);
                    continue;
                }
            }
        }
        cands.push(Cand {
            fid,
            exports,
            params,
            ret,
            in_bounds,
            visited,
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
    let reserved: HashSet<String> = cfg_reserved;
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
            reserved: &reserved,
            visiting: HashSet::new(),
            target: c.fid,
            fname: fname.clone(),
        };
        if !b.resolve_function(c.fid) {
            why!("not-self-contained {:?}", c.exports);
            continue;
        }
        if c.in_bounds {
            // helpers that got no container still must not read elements of arrays we cannot bound
            let unchecked = b
                .helpers
                .iter()
                .filter(|h| !c.visited.contains(h))
                .any(|&h| {
                    let none = HashSet::new();
                    readonly_ok(&an, &module, h, &none, &none, 0, &mut HashSet::new())
                        .is_none_or(|(total, _)| total > 0)
                });
            if unchecked {
                why!("unbounded-helper {:?}", c.exports);
                continue;
            }
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
            let text = text_with_edits(src, off(hf.lo), off(hf.hi), &b.edits);
            let old = hf.prop_name.clone().unwrap_or_default();
            let new = b.helper_name(*h, &old);
            prelude.push_str(&text.replacen(
                &format!("function {old}"),
                &format!("function {new}"),
                1,
            ));
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
            in_bounds_reads: c.in_bounds,
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
        let ts = with_declared_result(&ts, &fname, &c.ret.ts());
        out.push(AotClone {
            exports: c.exports,
            fn_name: fname,
            params: c.params.iter().map(|(_, p)| p.clone()).collect(),
            ret: c.ret.clone(),
            ts,
        });
    }
    Ok(out)
}
