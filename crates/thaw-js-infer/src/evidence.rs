//! Call-site evidence for the exported API of a package from programs that use it (smoke programs,
//! README examples, tests): which argument types are passed to `pkg.f(...)` / `f(...)` after
//! `import { f } from 'pkg'`. A *trusted contract* (it only covers the observed uses).

use crate::dts::DT;
use std::collections::HashMap;
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

pub const DEFAULT: &str = "<default>";

#[derive(Debug, Default, Clone)]
pub struct Evidence {
    /// exported name (or `<default>`) -> argument lists of the observed calls
    pub calls: HashMap<String, Vec<Vec<DT>>>,
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
    match e {
        a::Expr::Ident(i) if syms.contains_key(&*i.sym) => syms[&*i.sym].clone(),
        a::Expr::Paren(p) => dt_of(&p.expr, syms),
        a::Expr::Lit(a::Lit::Num(_)) => DT::Num,
        a::Expr::Lit(a::Lit::Str(_)) | a::Expr::Tpl(_) => DT::Str,
        a::Expr::Lit(a::Lit::Bool(_)) => DT::Bool,
        a::Expr::Lit(a::Lit::Null(_)) => DT::Null,
        a::Expr::Ident(i) if &*i.sym == "undefined" => DT::Undef,
        a::Expr::Unary(u) if matches!(u.op, a::UnaryOp::Minus | a::UnaryOp::Plus) => DT::Num,
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
        if !is_pkg(n.src.value.as_str().unwrap_or(""), self.pkg) {
            return;
        }
        for s in &n.specifiers {
            match s {
                a::ImportSpecifier::Default(d) => {
                    self.locals.insert(d.local.sym.to_string(), DEFAULT.into());
                }
                a::ImportSpecifier::Namespace(d) => {
                    self.locals.insert(d.local.sym.to_string(), "*".into());
                }
                a::ImportSpecifier::Named(d) => {
                    let imported = match &d.imported {
                        Some(a::ModuleExportName::Ident(i)) => i.sym.to_string(),
                        Some(a::ModuleExportName::Str(s)) => {
                            s.value.as_str().unwrap_or("").to_string()
                        }
                        None => d.local.sym.to_string(),
                    };
                    self.locals.insert(d.local.sym.to_string(), imported);
                }
            }
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
                    self.out.calls.entry(n).or_default().push(args);
                }
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
