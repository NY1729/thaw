fn rewrite_external_class_constructors(
    source: &str,
    classes: &[ClassConstructorRewrite],
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{Expr, MemberProp, NewExpr};
    use thaw_parser::common::Spanned;

    if classes.is_empty() {
        return Ok(source.to_string());
    }
    struct Finder<'a> {
        classes: &'a [(String, String)],
        removals: Vec<(u32, u32)>,
    }
    impl Visit for Finder<'_> {
        fn visit_new_expr(&mut self, expression: &NewExpr) {
            let matches = match expression.callee.as_ref() {
                Expr::Ident(class) => self
                    .classes
                    .iter()
                    .any(|(_, name)| name == class.sym.as_str()),
                Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                    (Expr::Ident(package), MemberProp::Ident(class)) => {
                        self.classes.iter().any(|(qualifier, name)| {
                            qualifier == package.sym.as_str() && name == class.sym.as_str()
                        })
                    }
                    _ => false,
                },
                _ => false,
            };
            if matches {
                self.removals
                    .push((expression.span().lo.0, expression.callee.span().lo.0));
            }
            expression.visit_children_with(self);
        }
    }
    let module = thaw_parser::parse_typescript(source)?;
    let mut finder = Finder {
        classes,
        removals: Vec::new(),
    };
    module.visit_with(&mut finder);
    let mut output = source.to_string();
    for (start, end) in finder.removals.into_iter().rev() {
        output.replace_range((start - 1) as usize..(end - 1) as usize, "");
    }
    Ok(output)
}

/// Rewrites `pkg.name(...)` call expressions in a user's own `.ts` source
/// to the package-qualified alias identifier `generate_registry_shims`'s
/// collision resolution actually emits for a colliding name (e.g.
/// `qs.stringify(x)` -> `qs_stringify(x)`), so a user can keep writing
/// the familiar namespace-qualified form even though Thaw has no real
/// object/member-call support backing it -- this is resolved *entirely*
/// as source-level syntax sugar, at this preprocessing step, not by the
/// compiler. `rewrites` is empty when there were no collisions at all,
/// in which case this returns `source` untouched without even parsing it.
///
/// Uses `swc_ecma_visit`'s `Visit` to walk the whole AST (a call can be
/// nested arbitrarily deep in an expression), unlike the top-level-only
/// walks elsewhere in this project (thaw-registry also uses a full AST walk
/// for JavaScript dependency discovery). Matched
/// spans are collected first and applied as one pass of text
/// substitution over the original source afterward, copying everything
/// else verbatim -- this project carries no general JS/TS code
/// generator, so re-printing from the AST isn't an option.
fn rewrite_qualified_calls(
    source: &str,
    rewrites: &[QualifiedCallRewrite],
) -> Result<String, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee, Expr, MemberProp};
    use thaw_parser::common::Spanned;

    if rewrites.is_empty() {
        return Ok(source.to_string());
    }

    struct Finder<'a> {
        rewrites: &'a [(String, String, String)],
        matches: Vec<(u32, u32, String)>,
    }
    impl Visit for Finder<'_> {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = &**callee {
                    if let (Expr::Ident(obj), MemberProp::Ident(prop)) =
                        (&*member.obj, &member.prop)
                    {
                        if let Some((_, _, alias)) = self.rewrites.iter().find(|(pkg, name, _)| {
                            pkg.as_str() == &*obj.sym && name.as_str() == &*prop.sym
                        }) {
                            let span = member.span();
                            self.matches.push((span.lo.0, span.hi.0, alias.clone()));
                        }
                    }
                }
            }
            call.visit_children_with(self);
        }
    }

    let (module, cm) = thaw_parser::parse_typescript_with_source_map(source)?;
    let mut finder = Finder {
        rewrites,
        matches: Vec::new(),
    };
    module.visit_with(&mut finder);

    if finder.matches.is_empty() {
        return Ok(source.to_string());
    }
    finder.matches.sort_by_key(|(lo, ..)| *lo);

    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (lo, hi, alias) in &finder.matches {
        let lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(*lo))
            .pos
            .0 as usize;
        let hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(*hi))
            .pos
            .0 as usize;
        out.push_str(&source[cursor..lo]);
        out.push_str(alias);
        cursor = hi;
    }
    out.push_str(&source[cursor..]);
    Ok(out)
}

