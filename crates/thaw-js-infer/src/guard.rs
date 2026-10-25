//! Entry guards for trusted contracts. A function whose parameter types come from a `.d.ts`,
//! observed calls or hints is only correct for callers that honour them. The guard is a JavaScript
//! predicate over the parameters; `annotate` (with `Config::guards`) turns such a function into a
//! dispatcher: when the guard holds the typed clone runs, otherwise the original body runs
//! unchanged, so a violated contract degrades to the dynamic path instead of a wrong result.

use crate::analyze::An;
use crate::ty::*;

impl An {
    /// JavaScript predicate that holds exactly when `v` is covered by `t`. `None` = not expressible
    /// (too deep, dead shape); the caller must not rely on `t` then.
    pub(crate) fn guard_js(
        &mut self,
        t: &Ty,
        v: &str,
        scope: usize,
        depth: usize,
    ) -> Option<String> {
        if depth > 4 {
            return None;
        }
        let atoms = match t {
            Ty::Bottom | Ty::Any(_) => return Some("true".into()),
            Ty::Set(s) => s.clone(),
        };
        let mut parts = Vec::new();
        for at in atoms {
            parts.push(match at {
                Atom::Undef => format!("{v} === undefined"),
                Atom::Null => format!("{v} === null"),
                Atom::Num => format!("typeof {v} === 'number'"),
                Atom::NLit(n) => format!("{v} === {n}"),
                Atom::Str => format!("typeof {v} === 'string'"),
                Atom::SLit(i) => format!("{v} === {:?}", self.lits[i as usize]),
                Atom::Bool => format!("typeof {v} === 'boolean'"),
                Atom::Fn(_) => format!("typeof {v} === 'function'"),
                Atom::Ns(_) => format!("typeof {v} === 'object'"),
                Atom::Arr(s) => {
                    let s = self.find(s);
                    let elem = match self.sites[s].clone() {
                        Site::Arr(e) => e,
                        _ => return None,
                    };
                    let e = format!("e{depth}");
                    let g = self.guard_js(&elem, &e, scope, depth + 1)?;
                    if g == "true" {
                        format!("Array.isArray({v})")
                    } else {
                        format!(
                            "(Array.isArray({v}) && {v}.every(function ({e}: any) {{ return {g}; }}))"
                        )
                    }
                }
                Atom::Obj(s) => {
                    let s = self.find(s);
                    if let Some(class) = self.site_class_of(s) {
                        // brand check; the class must be visible from the function
                        if self.st.resolve(scope, &class.1).as_ref() != Some(&class) {
                            return None;
                        }
                        format!("{v} instanceof {}", class.1)
                    } else {
                        let fields = match self.sites[s].clone() {
                            Site::Obj(m) => m,
                            _ => return None,
                        };
                        let mut cs =
                            vec![format!("typeof {v} === 'object'"), format!("{v} !== null")];
                        for (n, ft) in fields {
                            let ident = n
                                .chars()
                                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                                && !n.starts_with(|c: char| c.is_ascii_digit());
                            let access = if ident {
                                format!("{v}.{n}")
                            } else {
                                format!("{v}[{n:?}]")
                            };
                            let g = self.guard_js(&ft, &access, scope, depth + 1)?;
                            if g != "true" {
                                cs.push(g);
                            }
                        }
                        format!("({})", cs.join(" && "))
                    }
                }
            });
        }
        Some(if parts.len() == 1 {
            parts.remove(0)
        } else {
            format!("({})", parts.join(" || "))
        })
    }
}
