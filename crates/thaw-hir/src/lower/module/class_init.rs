// Which non-static fields of a class may still be unset (`undefined` in Node)
// once its constructor has returned. Purely syntactic and conservative: when
// in doubt a field is reported as maybe-uninitialised.

/// Finds a `return` outside nested functions.
struct ReturnFinder(bool);

impl Visit for ReturnFinder {
    fn visit_return_stmt(&mut self, _: &swc_ecma_ast::ReturnStmt) {
        self.0 = true;
    }
    fn visit_function(&mut self, _: &swc_ecma_ast::Function) {}
    fn visit_arrow_expr(&mut self, _: &swc_ecma_ast::ArrowExpr) {}
    fn visit_class(&mut self, _: &swc_ecma_ast::Class) {}
}

/// Field names assigned by `this.F = ...` (possibly chained) in `expr`.
fn this_assignments(expr: &Expr, assigned: &mut HashSet<String>) {
    let Expr::Assign(assign) = expr else {
        if let Expr::Paren(paren) = expr {
            this_assignments(&paren.expr, assigned);
        }
        return;
    };
    if assign.op != swc_ecma_ast::AssignOp::Assign {
        return;
    }
    this_assignments(&assign.right, assigned);
    let swc_ecma_ast::AssignTarget::Simple(swc_ecma_ast::SimpleAssignTarget::Member(member)) =
        &assign.left
    else {
        return;
    };
    if matches!(member.obj.as_ref(), Expr::This(_)) {
        if let swc_ecma_ast::MemberProp::Ident(ident) = &member.prop {
            assigned.insert(ident.sym.to_string());
        }
    }
}

/// Walks a statement sequence. Returns the fields assigned when control falls
/// off the end (`None` when every path diverges); `exits` collects the
/// assigned sets at each `return`.
fn walk_init_stmts(
    stmts: &[Stmt],
    mut assigned: HashSet<String>,
    exits: &mut Vec<HashSet<String>>,
) -> Option<HashSet<String>> {
    for stmt in stmts {
        match stmt {
            Stmt::Expr(expr) => this_assignments(&expr.expr, &mut assigned),
            Stmt::Block(block) => assigned = walk_init_stmts(&block.stmts, assigned, exits)?,
            Stmt::Throw(_) => return None,
            Stmt::Return(_) => {
                exits.push(assigned);
                return None;
            }
            Stmt::If(branch) => {
                let arm = |stmt: &Stmt, exits: &mut Vec<HashSet<String>>| {
                    walk_init_stmts(std::slice::from_ref(stmt), assigned.clone(), exits)
                };
                let consequent = arm(&branch.cons, exits);
                let alternate = match &branch.alt {
                    Some(alt) => arm(alt, exits),
                    None => Some(assigned.clone()),
                };
                assigned = match (consequent, alternate) {
                    (Some(a), Some(b)) => a.intersection(&b).cloned().collect(),
                    (Some(only), None) | (None, Some(only)) => only,
                    (None, None) => return None,
                };
            }
            other => {
                // Loops, `try`, `switch`, ...: assignments inside do not count,
                // but a `return` inside still exits with what is set so far.
                let mut finder = ReturnFinder(false);
                other.visit_with(&mut finder);
                if finder.0 {
                    exits.push(assigned.clone());
                }
            }
        }
    }
    Some(assigned)
}

/// Names of own instance fields of `class` that may be unset after
/// construction. Fields whose declared type already admits `undefined` are
/// filtered out by the caller (it knows the lowered type).
#[cfg(test)]
fn maybe_uninit_fields(class: &swc_ecma_ast::Class) -> HashSet<String> {
    maybe_uninit_fields_with(class, &|_| false)
}

/// Like `maybe_uninit_fields`; `bare_observed(field)` says whether a bare
/// `x: T;` of a constructor-less class may be observed as `undefined`.
fn maybe_uninit_fields_with(
    class: &swc_ecma_ast::Class,
    bare_observed: &dyn Fn(&str) -> bool,
) -> HashSet<String> {
    let mut assigned = HashSet::new();
    let constructor = class.body.iter().find_map(|member| match member {
        ClassMember::Constructor(constructor) => constructor.body.as_ref(),
        _ => None,
    });
    if let Some(body) = constructor {
        let mut exits = Vec::new();
        let end = walk_init_stmts(&body.stmts, HashSet::new(), &mut exits);
        let mut sets = exits.into_iter().chain(end);
        if let Some(first) = sets.next() {
            assigned = sets.fold(first, |acc, set| acc.intersection(&set).cloned().collect());
        }
    }
    // A constructor-less class with a bare `x: T;` (rejected by strict
    // TypeScript) keeps its zero layout unless the field may be observed, so
    // programs that never print or inspect it stay on the raw layout.
    let has_constructor = constructor.is_some();
    class
        .body
        .iter()
        .filter_map(|member| match member {
            ClassMember::ClassProp(property)
                if (property.definite
                    || has_constructor
                    || class_property_name(&property.key)
                        .is_ok_and(|name| bare_observed(&name)))
                    && !property.is_static
                    && property.value.is_none()
                    && !property.is_optional
                    && !property.declare
                    && !property.is_abstract =>
            {
                class_property_name(&property.key).ok()
            }
            _ => None,
        })
        .filter(|name| !assigned.contains(name.as_str()))
        .collect()
}

/// Field types that have a faithful `Optional` storage and lenient read
/// (everything whose declared type does not already admit `undefined`).
fn can_be_uninit(ty: &HirType) -> bool {
    matches!(
        ty,
        HirType::F64
            | HirType::I64 | HirType::BigInt
            | HirType::Bool
            | HirType::Str
            | HirType::Array(_)
            | HirType::Object(_)
            | HirType::Map(..)
            | HirType::Set(_)
            | HirType::Bytes
            | HirType::Function(..)
            | HirType::CallableFunction(..)
            | HirType::Json
            | HirType::Nullable(_)
    )
}
