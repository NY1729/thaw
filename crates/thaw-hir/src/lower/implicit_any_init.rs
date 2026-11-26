// Implicit-any (JS entries): an unannotated initializer that says nothing about later values is `any`.

/// Unannotated initializers whose first value says nothing about the later ones: `[]` is an
/// `any[]`, and a `let`/`var` starting as `null`/`undefined` is `any` (a later assignment widens it).
fn implicit_any_initializer_type(kind: VarDeclKind, init: &Expr) -> Option<HirType> {
    if !implicit_any() {
        return None;
    }
    match init {
        Expr::Array(array) if array.elems.is_empty() => Some(HirType::Array(Box::new(HirType::Json))),
        Expr::Lit(Lit::Null(_)) if kind != VarDeclKind::Const => Some(HirType::Json),
        Expr::Ident(ident) if kind != VarDeclKind::Const && ident.sym.as_ref() == "undefined" => {
            Some(HirType::Json)
        }
        _ => None,
    }
}
