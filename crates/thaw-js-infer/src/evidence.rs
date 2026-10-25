//! Call-site evidence for the exported API of a package from programs that use it (smoke programs,
//! README examples, tests): which argument types are passed to `pkg.f(...)` / `f(...)` after
//! `import { f } from 'pkg'`. A *trusted contract* (it only covers the observed uses).

use crate::dts::DT;
use std::collections::{HashMap, HashSet};
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

pub const DEFAULT: &str = "<default>";

#[derive(Debug, Default, Clone)]
pub struct Evidence {
    /// exported name (or `<default>`) -> argument lists of the observed calls
    pub calls: HashMap<String, Vec<Vec<DT>>>,
    /// the same calls typed for [`Evidence::proves`]: an empty array literal argument takes the
    /// element type of the parameter (`Arr(Union([]))`)
    pub proof_calls: HashMap<String, Vec<Vec<DT>>>,
    /// exports used other than as the callee of a direct call (`arr.map(f)`, `const g = f`, `ns.f`)
    pub value_uses: HashSet<String>,
    /// a namespace / default import is used as a value: any export may be reached
    pub opaque: bool,
    /// the program imports other local modules / uses `require` / `import()`: its calls are not all seen
    pub multi_file: bool,
    /// static result type of the package's functions as the compiler sees them (export name -> type)
    pub returns: HashMap<String, DT>,
}

/// Whether a value of the (syntactically evident) type `arg` is accepted where `want` is declared
/// without any conversion: equal primitives, arrays / flat records of conforming parts.
pub fn conforms(arg: &DT, want: &DT) -> bool {
    match (arg, want) {
        (DT::Num, DT::Num) | (DT::Str, DT::Str) | (DT::Bool, DT::Bool) => true,
        (DT::Arr(a), DT::Arr(w)) => match &**a {
            DT::Any => false,
            DT::Union(items) => items.iter().all(|i| conforms(i, w)),
            other => conforms(other, w),
        },
        (DT::Obj(a), DT::Obj(w)) => {
            a.len() == w.len()
                && w.iter()
                    .all(|(k, wt)| a.iter().any(|(ak, at)| ak == k && conforms(at, wt)))
        }
        (DT::Union(items), w) => !items.is_empty() && items.iter().all(|i| conforms(i, w)),
        _ => false,
    }
}

impl Evidence {
    /// Every reference to `name` in the observed program is a direct call whose arguments are
    /// evidently of the `want` types: the compiler's static types then prove the contract.
    pub fn proves(&self, names: &[&str], want: &[DT]) -> bool {
        if self.opaque || self.multi_file {
            return false;
        }
        names.iter().all(|n| {
            !self.value_uses.contains(*n)
                && self.proof_calls.get(*n).is_none_or(|calls| {
                    calls.iter().all(|args| {
                        args.len() == want.len()
                            && args.iter().zip(want).all(|(a, w)| conforms(a, w))
                    })
                })
        })
    }
}

type Syms = HashMap<String, DT>;

fn dt_of_ann(t: &a::TsType) -> DT {
    use a::TsKeywordTypeKind as K;
    match t {
        a::TsType::TsKeywordType(k) => match k.kind {
            K::TsNumberKeyword => DT::Num,
            K::TsStringKeyword => DT::Str,
            K::TsBooleanKeyword => DT::Bool,
            K::TsUndefinedKeyword | K::TsVoidKeyword => DT::Undef,
            K::TsNullKeyword => DT::Null,
            _ => DT::Any,
        },
        a::TsType::TsLitType(l) => match &l.lit {
            a::TsLit::Number(_) => DT::Num,
            a::TsLit::Str(_) => DT::Str,
            a::TsLit::Bool(_) => DT::Bool,
            _ => DT::Any,
        },
        a::TsType::TsArrayType(x) => DT::Arr(Box::new(dt_of_ann(&x.elem_type))),
        a::TsType::TsParenthesizedType(p) => dt_of_ann(&p.type_ann),
        a::TsType::TsUnionOrIntersectionType(a::TsUnionOrIntersectionType::TsUnionType(u)) => {
            DT::Union(u.types.iter().map(|x| dt_of_ann(x)).collect())
        }
        _ => DT::Any,
    }
}

fn dt_of(e: &a::Expr, syms: &Syms) -> DT {
    use a::BinaryOp as B;
    match e {
        a::Expr::Ident(i) if syms.contains_key(&*i.sym) => syms[&*i.sym].clone(),
        a::Expr::Paren(p) => dt_of(&p.expr, syms),
        a::Expr::Lit(a::Lit::Num(_)) => DT::Num,
        a::Expr::Lit(a::Lit::Str(_)) | a::Expr::Tpl(_) => DT::Str,
        a::Expr::Lit(a::Lit::Bool(_)) => DT::Bool,
        a::Expr::Lit(a::Lit::Null(_)) => DT::Null,
        a::Expr::Ident(i) if &*i.sym == "undefined" => DT::Undef,
        a::Expr::Unary(u) if matches!(u.op, a::UnaryOp::Minus | a::UnaryOp::Plus) => DT::Num,
        a::Expr::Unary(u) if u.op == a::UnaryOp::Bang => DT::Bool,
        a::Expr::Unary(u) if u.op == a::UnaryOp::TypeOf => DT::Str,
        a::Expr::Bin(b) => {
            let (l, r) = (dt_of(&b.left, syms), dt_of(&b.right, syms));
            match b.op {
                B::Sub
                | B::Mul
                | B::Div
                | B::Mod
                | B::Exp
                | B::BitAnd
                | B::BitOr
                | B::BitXor
                | B::LShift
                | B::RShift
                | B::ZeroFillRShift
                    if l == DT::Num && r == DT::Num =>
                {
                    DT::Num
                }
                B::Add if l == DT::Num && r == DT::Num => DT::Num,
                B::Add
                    if (l == DT::Str || r == DT::Str)
                        && matches!(l, DT::Str | DT::Num | DT::Bool)
                        && matches!(r, DT::Str | DT::Num | DT::Bool) =>
                {
                    DT::Str
                }
                B::Lt | B::LtEq | B::Gt | B::GtEq | B::EqEq | B::EqEqEq | B::NotEq | B::NotEqEq => {
                    DT::Bool
                }
                _ => DT::Any,
            }
        }
        a::Expr::Cond(c) => {
            let (x, y) = (dt_of(&c.cons, syms), dt_of(&c.alt, syms));
            if x == y {
                x
            } else {
                DT::Any
            }
        }
        a::Expr::Member(m) if matches!(&m.prop, a::MemberProp::Ident(p) if &*p.sym == "length") => {
            match dt_of(&m.obj, syms) {
                DT::Str | DT::Arr(_) => DT::Num,
                _ => DT::Any,
            }
        }
        a::Expr::Call(c) => match &c.callee {
            a::Callee::Expr(ce) => match &**ce {
                a::Expr::Ident(f) => match &*f.sym {
                    "parseFloat" | "parseInt" | "Number" => DT::Num,
                    "String" => DT::Str,
                    "Boolean" => DT::Bool,
                    name => syms.get(&format!("{name}()")).cloned().unwrap_or(DT::Any),
                },
                a::Expr::Member(m) => match (&*m.obj, &m.prop) {
                    (a::Expr::Ident(o), a::MemberProp::Ident(_)) if &*o.sym == "Math" => DT::Num,
                    // methods of strings / arrays / numbers whose result type is fixed
                    (obj, a::MemberProp::Ident(p)) if !matches!(obj, a::Expr::Ident(o) if syms.contains_key(&format!("@ns:{}", o.sym))) => {
                        match (dt_of(obj, syms), &*p.sym) {
                            (
                                DT::Str,
                                "repeat" | "toUpperCase" | "toLowerCase" | "trim" | "trimStart"
                                | "trimEnd" | "slice" | "substring" | "substr" | "padStart"
                                | "padEnd" | "concat" | "charAt" | "toString",
                            ) => DT::Str,
                            (
                                DT::Str,
                                "indexOf" | "lastIndexOf" | "charCodeAt" | "localeCompare",
                            ) => DT::Num,
                            (DT::Str, "includes" | "startsWith" | "endsWith") => DT::Bool,
                            (DT::Arr(_), "join") => DT::Str,
                            (DT::Arr(_), "indexOf" | "lastIndexOf") => DT::Num,
                            (DT::Num, "toFixed" | "toString") => DT::Str,
                            _ => DT::Any,
                        }
                    }
                    // `ns.f(..)` on a namespace import
                    (a::Expr::Ident(o), a::MemberProp::Ident(p))
                        if syms.contains_key(&format!("@ns:{}", o.sym)) =>
                    {
                        syms.get(&format!("@{}()", p.sym))
                            .cloned()
                            .unwrap_or(DT::Any)
                    }
                    _ => DT::Any,
                },
                _ => DT::Any,
            },
            _ => DT::Any,
        },
        a::Expr::Array(arr) => {
            let mut elems = Vec::new();
            for e in &arr.elems {
                match e {
                    Some(x) if x.spread.is_none() => elems.push(dt_of(&x.expr, syms)),
                    _ => return DT::Any,
                }
            }
            if elems.is_empty() {
                DT::Arr(Box::new(DT::Any))
            } else {
                DT::Arr(Box::new(DT::Union(elems)))
            }
        }
        a::Expr::Object(o) => {
            let mut fields = Vec::new();
            for p in &o.props {
                let a::PropOrSpread::Prop(p) = p else {
                    return DT::Any;
                };
                let a::Prop::KeyValue(kv) = &**p else {
                    return DT::Any;
                };
                let name = match &kv.key {
                    a::PropName::Ident(i) => i.sym.to_string(),
                    a::PropName::Str(s) => s.value.as_str().unwrap_or("").to_string(),
                    _ => return DT::Any,
                };
                fields.push((name, dt_of(&kv.value, syms)));
            }
            DT::Obj(fields)
        }
        _ => DT::Any,
    }
}

struct Collect<'a> {
    pkg: &'a str,
    /// local name -> exported name (`<default>` for the default import, `*` for a namespace)
    locals: HashMap<String, String>,
    /// names whose static type is evident from an annotation or a literal initialiser
    syms: Syms,
    out: &'a mut Evidence,
}

impl Collect<'_> {
    /// Names are tracked flat (no scoping): two different types for one name mean "unknown".
    fn note(&mut self, name: &str, t: DT) {
        match self.syms.get(name) {
            Some(old) if *old != t => {
                self.syms.insert(name.to_string(), DT::Any);
            }
            _ => {
                self.syms.insert(name.to_string(), t);
            }
        }
    }
}

fn is_pkg(spec: &str, pkg: &str) -> bool {
    spec == pkg || spec.strip_prefix("node:") == Some(pkg)
}

impl Visit for Collect<'_> {
    fn visit_import_decl(&mut self, n: &a::ImportDecl) {
        let spec = n.src.value.as_str().unwrap_or("");
        if spec.starts_with('.') || spec.starts_with('/') {
            self.out.multi_file = true;
        }
        if !is_pkg(spec, self.pkg) {
            return;
        }
        for s in &n.specifiers {
            match s {
                a::ImportSpecifier::Default(d) => {
                    self.locals.insert(d.local.sym.to_string(), DEFAULT.into());
                    if let Some(t) = self.out.returns.get(DEFAULT).cloned() {
                        self.syms.insert(format!("{}()", d.local.sym), t);
                    }
                }
                a::ImportSpecifier::Namespace(d) => {
                    self.locals.insert(d.local.sym.to_string(), "*".into());
                    self.syms.insert(format!("@ns:{}", d.local.sym), DT::Any);
                    for (name, t) in &self.out.returns {
                        self.syms.insert(format!("@{name}()"), t.clone());
                    }
                }
                a::ImportSpecifier::Named(d) => {
                    let imported = match &d.imported {
                        Some(a::ModuleExportName::Ident(i)) => i.sym.to_string(),
                        Some(a::ModuleExportName::Str(s)) => {
                            s.value.as_str().unwrap_or("").to_string()
                        }
                        None => d.local.sym.to_string(),
                    };
                    if let Some(t) = self.out.returns.get(&imported).cloned() {
                        self.syms.insert(format!("{}()", d.local.sym), t);
                    }
                    self.locals.insert(d.local.sym.to_string(), imported);
                }
            }
        }
    }
    fn visit_fn_decl(&mut self, n: &a::FnDecl) {
        if let Some(ret) = &n.function.return_type {
            let t = dt_of_ann(&ret.type_ann);
            self.note(&format!("{}()", n.ident.sym), t);
        }
        n.function.visit_with(self);
    }
    fn visit_expr(&mut self, e: &a::Expr) {
        match e {
            a::Expr::Ident(i) if self.locals.contains_key(&*i.sym) => {
                // a bare use of an imported binding (not the callee of a direct call)
                match self.locals[&*i.sym].as_str() {
                    "*" => self.out.opaque = true,
                    other => {
                        self.out.value_uses.insert(other.to_string());
                        if other == DEFAULT {
                            self.out.opaque = true;
                        }
                    }
                }
            }
            a::Expr::Member(m) => {
                if let a::Expr::Ident(o) = &*m.obj {
                    if self.locals.contains_key(&*o.sym) {
                        match &m.prop {
                            a::MemberProp::Ident(p) => {
                                self.out.value_uses.insert(p.sym.to_string());
                            }
                            _ => self.out.opaque = true,
                        }
                        return;
                    }
                }
                e.visit_children_with(self)
            }
            a::Expr::Call(c) if matches!(&c.callee, a::Callee::Import(_)) => {
                self.out.multi_file = true;
                e.visit_children_with(self)
            }
            _ => e.visit_children_with(self),
        }
    }
    fn visit_binding_ident(&mut self, n: &a::BindingIdent) {
        // `x: number` on a variable or parameter
        if let Some(ann) = &n.type_ann {
            self.note(&n.id.sym, dt_of_ann(&ann.type_ann));
        }
    }
    fn visit_var_declarator(&mut self, d: &a::VarDeclarator) {
        if let (a::Pat::Ident(i), Some(init)) = (&d.name, &d.init) {
            if i.type_ann.is_none() {
                let t = dt_of(init, &self.syms);
                if t != DT::Any {
                    self.note(&i.id.sym, t);
                }
            }
        }
        // const x = require('pkg') / const { a, b: c } = require('pkg')
        if let Some(a::Expr::Call(c)) = d.init.as_deref() {
            if let (a::Callee::Expr(ce), Some(arg)) = (&c.callee, c.args.first()) {
                if matches!(&**ce, a::Expr::Ident(i) if &*i.sym == "require") {
                    if let a::Expr::Lit(a::Lit::Str(s)) = &*arg.expr {
                        if is_pkg(s.value.as_str().unwrap_or(""), self.pkg) {
                            match &d.name {
                                a::Pat::Ident(i) => {
                                    self.locals.insert(i.id.sym.to_string(), DEFAULT.into());
                                }
                                a::Pat::Object(o) => {
                                    for p in &o.props {
                                        match p {
                                            a::ObjectPatProp::Assign(x) => {
                                                self.locals.insert(
                                                    x.key.sym.to_string(),
                                                    x.key.sym.to_string(),
                                                );
                                            }
                                            a::ObjectPatProp::KeyValue(kv) => {
                                                if let (a::PropName::Ident(k), a::Pat::Ident(v)) =
                                                    (&kv.key, &*kv.value)
                                                {
                                                    self.locals.insert(
                                                        v.id.sym.to_string(),
                                                        k.sym.to_string(),
                                                    );
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        d.visit_children_with(self);
    }
    fn visit_call_expr(&mut self, c: &a::CallExpr) {
        if let a::Callee::Expr(ce) = &c.callee {
            let name =
                match &**ce {
                    a::Expr::Ident(i) => self.locals.get(&*i.sym).map(|e| {
                        if e == "*" {
                            String::new()
                        } else {
                            e.clone()
                        }
                    }),
                    a::Expr::Member(m) => match (&*m.obj, &m.prop) {
                        (a::Expr::Ident(o), a::MemberProp::Ident(p))
                            if self.locals.contains_key(&*o.sym) =>
                        {
                            Some(p.sym.to_string())
                        }
                        _ => None,
                    },
                    _ => None,
                };
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                if c.args.iter().all(|x| x.spread.is_none()) {
                    let args = c.args.iter().map(|x| dt_of(&x.expr, &self.syms)).collect();
                    let proof = c
                        .args
                        .iter()
                        .map(|x| match &*x.expr {
                            a::Expr::Array(arr) if arr.elems.is_empty() => {
                                DT::Arr(Box::new(DT::Union(vec![])))
                            }
                            other => dt_of(other, &self.syms),
                        })
                        .collect();
                    self.out
                        .proof_calls
                        .entry(n.clone())
                        .or_default()
                        .push(proof);
                    self.out.calls.entry(n.clone()).or_default().push(args);
                } else {
                    self.out.value_uses.insert(n);
                }
            }
            if matches!(&**ce, a::Expr::Ident(i) if &*i.sym == "require") {
                self.out.multi_file = true;
            }
            // the callee itself is not a value use of an imported binding
            let direct = match &**ce {
                a::Expr::Ident(i) => self.locals.contains_key(&*i.sym),
                a::Expr::Member(m) => {
                    matches!((&*m.obj, &m.prop), (a::Expr::Ident(o), a::MemberProp::Ident(_)) if self.locals.contains_key(&*o.sym))
                }
                _ => false,
            };
            if direct {
                c.args.visit_with(self);
                return;
            }
        }
        c.visit_children_with(self);
    }
}

impl Evidence {
    /// Add the calls found in `src` (TypeScript or JavaScript); parse errors are ignored.
    pub fn add_program(&mut self, pkg: &str, src: &str) {
        let module =
            thaw_parser::parse_typescript(src).or_else(|_| thaw_parser::parse_javascript(src));
        let Ok(module) = module else { return };
        let mut c = Collect {
            pkg,
            locals: HashMap::new(),
            syms: HashMap::new(),
            out: self,
        };
        module.visit_with(&mut c);
    }

    /// Add the fenced js/ts code blocks of a README.
    pub fn add_readme(&mut self, pkg: &str, md: &str) {
        let mut in_block = false;
        let mut buf = String::new();
        for line in md.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("```") {
                if in_block {
                    self.add_program(pkg, &buf);
                    buf.clear();
                    in_block = false;
                } else {
                    in_block = matches!(
                        rest.trim(),
                        "js" | "javascript" | "ts" | "typescript" | "mjs" | "cjs"
                    );
                }
            } else if in_block {
                buf.push_str(line);
                buf.push('\n');
            }
        }
    }
}
