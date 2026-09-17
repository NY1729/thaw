/// Scoring-only information about a type that failed to classify as a
/// native `HirType` at all -- whether the presence or absence of a
/// specific object-literal key is enough to tell it apart from a
/// sibling overload's own equally-unresolvable parameter type. Real
/// example: `tar`'s own `options.js`, `type TarOptionsWithAliasesFile =
/// (TarOptionsWithAliases & { file: string }) | (TarOptionsWithAliases
/// & { f: string })` vs `type TarOptionsWithAliasesNoFile =
/// TarOptionsWithAliases & { f?: undefined; file?: undefined }` -- two
/// ~50-optional-field option interfaces distinguished *only* by whether
/// `file`/`f` is required or forced absent. Neither interface resolves
/// as a native `HirType::Object` (`TarOptions` itself has several
/// fields -- `Map<string, Date>`, a template-literal-typed field, ... --
/// thaw-bridge's own full classifier can't represent at all, and one
/// unclassifiable field fails the *whole* interface, all-or-nothing --
/// see `resolve_interface`'s own doc comment), so both parameters widen
/// to the same opaque `Json` and ordinary type-based overload scoring
/// can't tell them apart. This is computed independently of
/// `classify_ts_type`/`resolve_interfaces`, walking each interface's raw
/// `extends` chain and property signatures directly and tolerating a
/// field it can't otherwise make sense of (only *presence*, not the
/// field's own value type, is ever relevant here) -- checked separately,
/// against the call site's own object-literal argument, by thaw-cli's
/// overload dispatch.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FieldConstraints {
    /// Satisfied if the call-site object literal has *every* key in at
    /// least one of these groups (OR across groups, AND within one) --
    /// a single-element `Vec` for a plain interface; a union of two
    /// alternative shapes (tar's own `file`-or-`f` alias pair)
    /// contributes one group per member.
    pub required_alternatives: Vec<Vec<String>>,
    /// Disqualifying if the call-site literal has *any* of these keys
    /// (a field this shape's own type declares always-absent, e.g.
    /// `file?: undefined`).
    pub excluded: Vec<String>,
}

impl FieldConstraints {
    fn is_empty(&self) -> bool {
        self.required_alternatives.is_empty() && self.excluded.is_empty()
    }

    fn merge(mut self, other: FieldConstraints) -> FieldConstraints {
        self.required_alternatives
            .extend(other.required_alternatives);
        self.excluded.extend(other.excluded);
        self
    }
}

/// Required/excluded field names contributed directly by one
/// interface's own property signatures (not its `extends` bases,
/// merged in separately by the caller) -- required if not marked `?`;
/// excluded if marked `?` and typed as the literal `undefined` keyword
/// (the `{ file?: undefined }` idiom); anything else (an ordinary
/// optional field with a real type) contributes to neither, and a
/// non-property member (a method/index/call signature) is simply
/// skipped rather than aborting the whole interface -- unlike
/// `resolve_interface`, which needs a fully faithful `HirType::Object`
/// and so cannot tolerate that.
fn own_field_constraints(members: &[TsTypeElement]) -> FieldConstraints {
    let mut required = Vec::new();
    let mut excluded = Vec::new();
    for member in members {
        let TsTypeElement::TsPropertySignature(prop) = member else {
            continue;
        };
        let Some(field_name) = type_property_name(&prop.key) else {
            continue;
        };
        if !prop.optional {
            required.push(field_name);
            continue;
        }
        let is_bare_undefined = prop.type_ann.as_ref().is_some_and(|ann| {
            matches!(
                ann.type_ann.as_ref(),
                TsType::TsKeywordType(keyword) if keyword.kind == TsKeywordTypeKind::TsUndefinedKeyword
            )
        });
        if is_bare_undefined {
            excluded.push(field_name);
        }
    }
    let required_alternatives = if required.is_empty() {
        Vec::new()
    } else {
        vec![required]
    };
    FieldConstraints {
        required_alternatives,
        excluded,
    }
}

/// Resolves `name` as an interface (via `raw_interfaces`, the same raw
/// name -> `&TsInterfaceDecl` map `extract_const_call_signature_decls`
/// already builds as `all_interface_decls_by_name`) and merges its own
/// field constraints with every base in its `extends` chain, tolerating
/// an extends target this can't resolve (a generic instantiation, a
/// non-plain-name expression, an unknown name) by simply not
/// contributing anything from it rather than aborting -- unlike
/// `resolve_interface`, whose own all-or-nothing merge this deliberately
/// does not reuse (see `FieldConstraints`'s own doc comment for why).
/// `visited` guards against a self-referential `extends` cycle.
fn interface_field_constraints(
    name: &str,
    raw_interfaces: &HashMap<String, &TsInterfaceDecl>,
    visited: &mut HashSet<String>,
) -> Option<FieldConstraints> {
    if !visited.insert(name.to_string()) {
        return None;
    }
    let interface = raw_interfaces.get(name)?;
    let mut constraints = own_field_constraints(&interface.body.body);
    for base in &interface.extends {
        if base.type_args.is_some() {
            continue;
        }
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            continue;
        };
        if let Some(base_constraints) =
            interface_field_constraints(&base_ident.sym, raw_interfaces, visited)
        {
            constraints = constraints.merge(base_constraints);
        }
    }
    (!constraints.is_empty()).then_some(constraints)
}

/// Computes [`FieldConstraints`] for `ty`, or `None` if it carries no
/// checkable field-presence information at all (same "no information"
/// outcome as an ordinary unresolvable type today -- this is purely
/// additive, never a replacement for `classify_ts_type`). `raw_
/// interfaces` is the raw name -> `&TsInterfaceDecl` map (see
/// `interface_field_constraints`); `local_type_aliases` is the raw,
/// unresolved name -> `TsType` map for chasing a `type X = ...` alias
/// (mirrors `resolve_local_callable_fn_types`'s identical two-map
/// lookup pattern). `visited` guards against a self-referential alias
/// cycle.
pub fn field_constraints(
    ty: &TsType,
    raw_interfaces: &HashMap<String, &TsInterfaceDecl>,
    local_type_aliases: &HashMap<String, &TsType>,
    visited: &mut HashSet<String>,
) -> Option<FieldConstraints> {
    match ty {
        TsType::TsParenthesizedType(parenthesized) => field_constraints(
            &parenthesized.type_ann,
            raw_interfaces,
            local_type_aliases,
            visited,
        ),
        // An inline object type literal (`{ file: string }`, `{ f?:
        // undefined; file?: undefined }`) -- the shape a union/
        // intersection alias built from anonymous refinements uses
        // directly, rather than naming another interface. Real
        // example: tar's own `type TarOptionsWithAliasesFile =
        // (TarOptionsWithAliases & { file: string }) | (TarOptions
        // WithAliases & { f: string })`.
        TsType::TsTypeLit(lit) => {
            let constraints = own_field_constraints(&lit.members);
            (!constraints.is_empty()).then_some(constraints)
        }
        TsType::TsTypeRef(ty_ref) => {
            let name = match &ty_ref.type_name {
                TsEntityName::Ident(ident) => ident.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };
            if let Some(constraints) =
                interface_field_constraints(&name, raw_interfaces, &mut HashSet::new())
            {
                return Some(constraints);
            }
            if !visited.insert(name.clone()) {
                return None;
            }
            match local_type_aliases.get(name.as_str()) {
                Some(aliased) => {
                    field_constraints(aliased, raw_interfaces, local_type_aliases, visited)
                }
                None => None,
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let members = intersection
                .types
                .iter()
                .filter_map(|member| {
                    field_constraints(member, raw_interfaces, local_type_aliases, visited)
                })
                .collect::<Vec<_>>();
            if members.is_empty() {
                return None;
            }
            let mut required_alternatives = Vec::new();
            let mut excluded = Vec::new();
            for member in members {
                if required_alternatives.is_empty() {
                    required_alternatives = member.required_alternatives;
                } else if !member.required_alternatives.is_empty() {
                    // Every member's own requirement must hold at once --
                    // merge each existing alternative group with each of
                    // this member's own groups (a cross product; in every
                    // real shape seen so far, each side contributes just
                    // one group, so this is a simple concatenation).
                    required_alternatives = required_alternatives
                        .iter()
                        .flat_map(|existing| {
                            member.required_alternatives.iter().map(move |addition| {
                                existing
                                    .iter()
                                    .cloned()
                                    .chain(addition.iter().cloned())
                                    .collect()
                            })
                        })
                        .collect();
                }
                excluded.extend(member.excluded);
            }
            Some(FieldConstraints {
                required_alternatives,
                excluded,
            })
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let members = union
                .types
                .iter()
                .filter_map(|member| {
                    field_constraints(member, raw_interfaces, local_type_aliases, visited)
                })
                .collect::<Vec<_>>();
            if members.is_empty() {
                return None;
            }
            let required_alternatives = members
                .iter()
                .flat_map(|member| member.required_alternatives.iter().cloned())
                .collect();
            // Only a key every resolvable alternative agrees is forbidden
            // is a hard exclusion for the whole union -- one alternative
            // that doesn't mention it at all can't rule it out.
            let excluded = members
                .first()
                .map(|first| {
                    first
                        .excluded
                        .iter()
                        .filter(|key| members.iter().all(|member| member.excluded.contains(key)))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            Some(FieldConstraints {
                required_alternatives,
                excluded,
            })
        }
        _ => None,
    }
}
