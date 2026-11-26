// Implicit-any (JS entries) fallbacks: a type inference could not resolve is `Json`, never an error.

/// Replace every `Dynamic` left in `ty` after the call-site fixpoint with `Json`.
fn dynamic_to_json(ty: &HirType) -> HirType {
    let b = |t: &HirType| Box::new(dynamic_to_json(t));
    match ty {
        HirType::Dynamic => HirType::Json,
        HirType::Promise(t) => HirType::Promise(b(t)),
        HirType::Array(t) => HirType::Array(b(t)),
        HirType::Optional(t) => HirType::Optional(b(t)),
        HirType::Nullable(t) => HirType::Nullable(b(t)),
        HirType::Nullish(t) => HirType::Nullish(b(t)),
        HirType::Tuple(ts) => HirType::Tuple(ts.iter().map(dynamic_to_json).collect()),
        HirType::Union(ts) => HirType::Union(ts.iter().map(dynamic_to_json).collect()),
        HirType::Object(fields) => HirType::Object(
            fields.iter().map(|(n, t)| (n.clone(), dynamic_to_json(t))).collect(),
        ),
        HirType::Function(ps, r) => {
            HirType::Function(ps.iter().map(dynamic_to_json).collect(), b(r))
        }
        other => other.clone(),
    }
}
