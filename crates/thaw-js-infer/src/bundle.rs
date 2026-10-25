//! Registry bundle layout (`__thaw_bundle_factories` / `__thaw_bundle_require_maps`): which
//! factory function is which module and what `require('./x')` inside it resolves to.

use std::collections::HashMap;
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

#[derive(Debug, Default, Clone)]
pub struct Bundle {
    pub entry: String,
    /// factory key -> span of its function
    pub factories: Vec<(String, (u32, u32))>,
    /// factory key -> (require spec -> target factory key)
    pub require_maps: HashMap<String, HashMap<String, String>>,
}

fn prop_key(p: &a::PropName) -> Option<String> {
    match p {
        a::PropName::Ident(i) => Some(i.sym.to_string()),
        a::PropName::Str(s) => s.value.as_str().map(str::to_string),
        _ => None,
    }
}

#[derive(Default)]
struct Find {
    b: Bundle,
}

impl Visit for Find {
    fn visit_var_declarator(&mut self, d: &a::VarDeclarator) {
        if let (a::Pat::Ident(id), Some(init)) = (&d.name, &d.init) {
            match (&*id.id.sym, &**init) {
                ("__thaw_bundle_entry_key", a::Expr::Lit(a::Lit::Str(s))) => {
                    self.b.entry = s.value.as_str().unwrap_or("").to_string()
                }
                ("__thaw_bundle_factories", a::Expr::Object(o)) => {
                    for p in &o.props {
                        if let a::PropOrSpread::Prop(p) = p {
                            if let a::Prop::KeyValue(kv) = &**p {
                                if let (Some(k), a::Expr::Fn(f)) = (prop_key(&kv.key), &*kv.value) {
                                    self.b
                                        .factories
                                        .push((k, (f.function.span.lo.0, f.function.span.hi.0)));
                                }
                            }
                        }
                    }
                }
                (
                    "__thaw_bundle_require_maps" | "__thaw_bundle_import_maps",
                    a::Expr::Object(o),
                ) => {
                    for p in &o.props {
                        let a::PropOrSpread::Prop(p) = p else {
                            continue;
                        };
                        let a::Prop::KeyValue(kv) = &**p else {
                            continue;
                        };
                        let (Some(k), a::Expr::Object(inner)) = (prop_key(&kv.key), &*kv.value)
                        else {
                            continue;
                        };
                        let m = self.b.require_maps.entry(k).or_default();
                        for ip in &inner.props {
                            let a::PropOrSpread::Prop(ip) = ip else {
                                continue;
                            };
                            let a::Prop::KeyValue(ikv) = &**ip else {
                                continue;
                            };
                            if let (Some(sk), a::Expr::Lit(a::Lit::Str(sv))) =
                                (prop_key(&ikv.key), &*ikv.value)
                            {
                                m.insert(sk, sv.value.as_str().unwrap_or("").to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        d.visit_children_with(self);
    }
}

pub fn find(module: &a::Module) -> Option<Bundle> {
    let mut f = Find::default();
    module.visit_with(&mut f);
    (!f.b.factories.is_empty()).then_some(f.b)
}
