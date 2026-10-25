//! `.d.ts` hints: declared parameter / return types for functions and methods, keyed by name (and
//! owning class / interface). The declarations are a *trusted contract* (like TypeScript itself): a
//! function typed from them needs an entry guard when called from untyped code (design s.5.3).

use std::collections::HashMap;
use thaw_parser::ast as a;

/// A declared type, independent of the analysis' allocation sites.
#[derive(Clone, Debug, PartialEq)]
pub enum DT {
    Num,
    Str,
    Bool,
    Undef,
    Null,
    /// `any`, `unknown`, generics, functions, unsupported shapes
    Any,
    Arr(Box<DT>),
    Union(Vec<DT>),
    Obj(Vec<(String, DT)>),
}

#[derive(Clone, Debug)]
pub struct Sig {
    pub owner: Option<String>,
    pub params: Vec<DT>,
    pub ret: DT,
}

#[derive(Clone, Debug, Default)]
pub struct DtsIndex {
    pub sigs: HashMap<String, Vec<Sig>>,
}

struct Ctx {
    aliases: HashMap<String, a::TsType>,
    ifaces: HashMap<String, Vec<a::TsTypeElement>>,
}

fn last_name(n: &a::TsEntityName) -> String {
    match n {
        a::TsEntityName::Ident(i) => i.sym.to_string(),
        a::TsEntityName::TsQualifiedName(q) => q.right.sym.to_string(),
    }
}

fn key_name(k: &a::Expr) -> Option<String> {
    match k {
        a::Expr::Ident(i) => Some(i.sym.to_string()),
        a::Expr::Lit(a::Lit::Str(s)) => s.value.as_str().map(str::to_string),
        _ => None,
    }
}

fn collect(items: &[a::ModuleItem], cx: &mut Ctx, decls: &mut Vec<a::Decl>) {
    for it in items {
        let d = match it {
            a::ModuleItem::Stmt(a::Stmt::Decl(d)) => d,
            a::ModuleItem::ModuleDecl(a::ModuleDecl::ExportDecl(e)) => &e.decl,
            _ => continue,
        };
        match d {
            a::Decl::TsTypeAlias(t) => {
                cx.aliases
                    .insert(t.id.sym.to_string(), (*t.type_ann).clone());
            }
            a::Decl::TsInterface(i) => {
                cx.ifaces
                    .entry(i.id.sym.to_string())
                    .or_default()
                    .extend(i.body.body.clone());
                decls.push(d.clone());
            }
            a::Decl::TsModule(m) => {
                let mut body = m.body.as_ref();
                while let Some(b) = body {
                    match b {
                        a::TsNamespaceBody::TsModuleBlock(blk) => {
                            collect(&blk.body, cx, decls);
                            break;
                        }
                        a::TsNamespaceBody::TsNamespaceDecl(n) => body = Some(&n.body),
                    }
                }
            }
            a::Decl::Fn(_) | a::Decl::Class(_) => decls.push(d.clone()),
            _ => {}
        }
    }
}

impl Ctx {
    fn ann(&self, t: &Option<Box<a::TsTypeAnn>>, depth: usize) -> DT {
        match t {
            Some(t) => self.ty(&t.type_ann, depth),
            None => DT::Any,
        }
    }

    fn members(&self, els: &[a::TsTypeElement], depth: usize) -> DT {
        let mut fields = Vec::new();
        for e in els {
            match e {
                a::TsTypeElement::TsPropertySignature(p) => {
                    let (Some(k), false) = (key_name(&p.key), p.computed) else {
                        return DT::Any;
                    };
                    let mut t = self.ann(&p.type_ann, depth + 1);
                    if p.optional {
                        t = DT::Union(vec![t, DT::Undef]);
                    }
                    fields.push((k, t));
                }
                // methods, index/call signatures: not a plain data struct
                _ => return DT::Any,
            }
        }
        DT::Obj(fields)
    }

    fn ty(&self, t: &a::TsType, depth: usize) -> DT {
        use a::TsKeywordTypeKind as K;
        if depth > 6 {
            return DT::Any;
        }
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
            a::TsType::TsArrayType(x) => DT::Arr(Box::new(self.ty(&x.elem_type, depth + 1))),
            a::TsType::TsParenthesizedType(p) => self.ty(&p.type_ann, depth),
            a::TsType::TsTypeOperator(o) => self.ty(&o.type_ann, depth),
            a::TsType::TsUnionOrIntersectionType(a::TsUnionOrIntersectionType::TsUnionType(u)) => {
                DT::Union(u.types.iter().map(|x| self.ty(x, depth + 1)).collect())
            }
            a::TsType::TsTypeLit(l) => self.members(&l.members, depth),
            a::TsType::TsTypeRef(r) => {
                let name = last_name(&r.type_name);
                let args = r.type_params.as_ref().map(|p| p.params.len()).unwrap_or(0);
                match name.as_str() {
                    "Array" | "ReadonlyArray" if args == 1 => {
                        let p = &r.type_params.as_ref().unwrap().params[0];
                        DT::Arr(Box::new(self.ty(p, depth + 1)))
                    }
                    _ if args > 0 => DT::Any, // generic instantiation
                    _ => {
                        if let Some(al) = self.aliases.get(&name) {
                            self.ty(al, depth + 1)
                        } else if let Some(els) = self.ifaces.get(&name) {
                            self.members(els, depth + 1)
                        } else {
                            DT::Any
                        }
                    }
                }
            }
            _ => DT::Any,
        }
    }

    fn fn_param(&self, p: &a::Pat) -> DT {
        match p {
            a::Pat::Ident(b) => {
                let t = self.ann(&b.type_ann, 0);
                if b.id.optional {
                    DT::Union(vec![t, DT::Undef])
                } else {
                    t
                }
            }
            _ => DT::Any,
        }
    }

    fn ts_param(&self, p: &a::TsFnParam) -> DT {
        match p {
            a::TsFnParam::Ident(b) => {
                let t = self.ann(&b.type_ann, 0);
                if b.id.optional {
                    DT::Union(vec![t, DT::Undef])
                } else {
                    t
                }
            }
            _ => DT::Any,
        }
    }
}

impl DtsIndex {
    pub fn parse(src: &str) -> Result<DtsIndex, String> {
        let module = thaw_parser::parse_declarations(src)?;
        let mut cx = Ctx {
            aliases: HashMap::new(),
            ifaces: HashMap::new(),
        };
        let mut decls = Vec::new();
        collect(&module.body, &mut cx, &mut decls);
        let mut idx = DtsIndex::default();
        let mut add = |name: String, owner: Option<String>, params: Vec<DT>, ret: DT| {
            idx.sigs
                .entry(name)
                .or_default()
                .push(Sig { owner, params, ret });
        };
        for d in &decls {
            match d {
                a::Decl::Fn(f) => {
                    let params = f
                        .function
                        .params
                        .iter()
                        .map(|p| cx.fn_param(&p.pat))
                        .collect();
                    add(
                        f.ident.sym.to_string(),
                        None,
                        params,
                        cx.ann(&f.function.return_type, 0),
                    );
                }
                a::Decl::Class(c) => {
                    let owner = c.ident.sym.to_string();
                    for m in &c.class.body {
                        if let a::ClassMember::Constructor(c) = m {
                            let params = c
                                .params
                                .iter()
                                .map(|p| match p {
                                    a::ParamOrTsParamProp::Param(p) => cx.fn_param(&p.pat),
                                    _ => DT::Any,
                                })
                                .collect();
                            add("constructor".into(), Some(owner.clone()), params, DT::Undef);
                        }
                        if let a::ClassMember::Method(m) = m {
                            let Some(k) = (match &m.key {
                                a::PropName::Ident(i) => Some(i.sym.to_string()),
                                a::PropName::Str(s) => s.value.as_str().map(str::to_string),
                                _ => None,
                            }) else {
                                continue;
                            };
                            let params = m
                                .function
                                .params
                                .iter()
                                .map(|p| cx.fn_param(&p.pat))
                                .collect();
                            add(
                                k,
                                Some(owner.clone()),
                                params,
                                cx.ann(&m.function.return_type, 0),
                            );
                        }
                    }
                }
                a::Decl::TsInterface(i) => {
                    let owner = i.id.sym.to_string();
                    for e in &i.body.body {
                        if let a::TsTypeElement::TsMethodSignature(m) = e {
                            let Some(k) = key_name(&m.key) else { continue };
                            let params = m.params.iter().map(|p| cx.ts_param(p)).collect();
                            add(k, Some(owner.clone()), params, cx.ann(&m.type_ann, 0));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(idx)
    }

    /// Signatures for a function called `name` owned by `owner` (a class / interface name, or `None`
    /// for a free function / namespace member). Generic signatures yield `DT::Any` parameters.
    pub fn lookup(&self, name: &str, owner: Option<&str>) -> Vec<&Sig> {
        self.sigs
            .get(name)
            .map(|v| v.iter().filter(|s| s.owner.as_deref() == owner).collect())
            .unwrap_or_default()
    }
}
