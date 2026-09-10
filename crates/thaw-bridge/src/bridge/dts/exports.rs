/// Parses a `.d.ts` source string and extracts every top-level function
/// signature (`declare function foo(...): T;` and
/// `export declare function foo(...): T;` -- `.d.ts` files don't have
/// function bodies to begin with, so plain `export function foo(...): T;`
/// is equally ambient here). `interface` declarations are resolved first
/// (see `resolve_interfaces`) so a signature using one classifies as
/// `Native` just like an inline `{ ... }` type literal would. Anything else
/// at the top level (classes, `const`, re-exports, ...) is silently
/// skipped: this is a function-signature extractor, not a full `.d.ts`
/// model.
pub fn parse_dts(source: &str) -> Result<Vec<DtsFunction>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let mut functions = module
        .body
        .iter()
        .flat_map(extract_fn_decls)
        .map(|(name, func)| {
            let name = name.rsplit('.').next().unwrap_or(name);
            lower_dts_function(name, func, &interfaces, &generic_interfaces)
        })
        .collect::<Vec<_>>();
    if let Some(target) = export_assignment_interface_name(&module) {
        functions.extend(
            module
                .body
                .iter()
                .flat_map(|item| extract_interface_method_decls(item, &target))
                .map(|(name, method)| {
                    lower_dts_method_signature(&name, method, &interfaces, &generic_interfaces)
                }),
        );
    }
    let call_signature_interfaces = all_interface_decls_by_name(&module);
    let local_type_aliases = all_type_alias_decls_by_name(&module);
    functions.extend(
        module
            .body
            .iter()
            .flat_map(|item| {
                extract_const_call_signature_decls(
                    item,
                    &call_signature_interfaces,
                    &local_type_aliases,
                )
            })
            .map(|(name, signature)| match signature {
                CallableConstSignature::Interface(call) => {
                    lower_dts_call_signature(&name, call, &interfaces, &generic_interfaces)
                }
                CallableConstSignature::Direct(function) => {
                    lower_dts_fn_type(&name, function, &interfaces, &generic_interfaces)
                }
            }),
    );
    Ok(functions)
}

/// Extracts typed, non-callable top-level value declarations. Callable
/// `const`s are already returned by [`parse_dts`] and are excluded here.
pub fn parse_dts_values(source: &str) -> Result<Vec<DtsValue>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let callable_objects = module
        .body
        .iter()
        .flat_map(extract_interface_decls)
        .filter(|interface| {
            interface.body.body.iter().any(|member| {
                matches!(
                    member,
                    TsTypeElement::TsPropertySignature(_) | TsTypeElement::TsMethodSignature(_)
                )
            })
        })
        .map(|interface| interface.id.sym.to_string())
        .collect::<HashSet<_>>();
    let callable = parse_dts(source)?
        .into_iter()
        .map(|function| function.name)
        .collect::<HashSet<_>>();
    let class_names = parse_dts_classes(source)?
        .into_iter()
        .map(|class| class.name)
        .collect::<HashSet<_>>();
    let mut values = Vec::new();
    for item in &module.body {
        let declaration = match item {
            ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(declaration))) => {
                declaration.as_ref()
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Var(declaration) => declaration.as_ref(),
                _ => continue,
            },
            _ => continue,
        };
        for declarator in &declaration.decls {
            let Pat::Ident(binding) = &declarator.name else {
                continue;
            };
            let name = binding.id.sym.to_string();
            let Some(annotation) = &binding.type_ann else {
                continue;
            };
            if callable.contains(&name) {
                let callable_object = matches!(
                    annotation.type_ann.as_ref(),
                    TsType::TsTypeRef(reference)
                        if matches!(&reference.type_name, TsEntityName::Ident(interface)
                            if callable_objects.contains(interface.sym.as_str()))
                );
                if callable_object {
                    values.push(DtsValue {
                        name,
                        ty: DtsType::Native(HirType::JsValue),
                    });
                }
                continue;
            }
            let mut ty = resolve_ts_type_with_substitution(
                &annotation.type_ann,
                &HashMap::new(),
                &interfaces,
                &generic_interfaces,
                &mut Vec::new(),
            );
            if matches!(&ty, DtsType::Unsupported(_)) {
                let referenced = match annotation.type_ann.as_ref() {
                    TsType::TsTypeRef(reference) => match &reference.type_name {
                        TsEntityName::Ident(ident) => Some(ident.sym.as_str()),
                        TsEntityName::TsQualifiedName(qualified) => Some(qualified.right.sym.as_str()),
                    },
                    _ => None,
                };
                if referenced.is_some_and(|name| class_names.contains(name)) {
                    ty = DtsType::Native(HirType::JsValue);
                }
            }
            values.push(DtsValue { name, ty });
        }
    }
    Ok(values)
}

/// Every interface declared anywhere in `module`, by its own bare name --
/// unlike `resolve_interfaces`'s internal `raw` map, this doesn't split
/// generic from non-generic interfaces (a call-signature interface may
/// carry its own unrelated generic parameter, e.g. drizzle-orm's
/// `SQLiteTableFn<TSchema extends string | undefined = undefined>`, which
/// is irrelevant to extracting its call signatures below). Used only to
/// look an interface up by name for `extract_const_call_signature_decls`;
/// not a replacement for `resolve_interfaces`'s own classification map.
fn all_interface_decls_by_name(module: &Module) -> HashMap<String, &TsInterfaceDecl> {
    let mut map = HashMap::new();
    for iface in module.body.iter().flat_map(extract_interface_decls) {
        map.entry(iface.id.sym.to_string()).or_insert(iface);
    }
    map
}

/// Every *non-generic* type alias declared anywhere in `module`, by its
/// own bare name -- a generic alias's own type parameter would need
/// substitution to resolve at all, which `resolve_local_callable_fn_types`
/// below doesn't attempt (mirrors `resolve_interfaces`'s identical
/// generic/non-generic split for interfaces). Used only to look a type
/// alias up by name when resolving a `declare const`'s own type, not a
/// replacement for `resolve_interfaces`'s own classification map.
fn all_type_alias_decls_by_name(module: &Module) -> HashMap<String, &TsType> {
    let mut map = HashMap::new();
    for alias in module.body.iter().flat_map(extract_type_alias_decls) {
        if alias.type_params.is_some() {
            continue;
        }
        map.entry(alias.id.sym.to_string())
            .or_insert(alias.type_ann.as_ref());
    }
    map
}

/// Resolves `ty` to zero or more direct function-type call signatures,
/// following a chain of local (bare or exported) non-generic type-alias
/// references and unwrapping an intersection into each of its own
/// operands. Real example: uuid's own `.d.ts` (via `@types/uuid`), `type
/// v4 = v4Buffer & v4String;`, where `v4Buffer`/`v4String` are themselves
/// further local aliases each resolving to a direct (possibly its own
/// separately-generic) function type -- `v1`/`v3`/`v5`/`v6`/`v7`/
/// `parse`/`stringify`/etc. all use the identical two-alias-intersection
/// shape. `visited` guards against a self-referential alias cycle. Real
/// `.d.ts` shapes seen so far never need anything deeper than this (a
/// plain reference, or an intersection of references/direct function
/// types) -- a union, mapped type, etc. isn't unwrapped, and just
/// contributes no call signatures (same as any other unresolvable type).
fn resolve_local_callable_fn_types<'a>(
    ty: &'a TsType,
    aliases: &HashMap<String, &'a TsType>,
    visited: &mut HashSet<String>,
) -> Vec<&'a TsFnType> {
    match ty {
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            vec![function]
        }
        TsType::TsTypeRef(ty_ref) => {
            let name = match &ty_ref.type_name {
                TsEntityName::Ident(ident) => ident.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };
            if !visited.insert(name.clone()) {
                return Vec::new();
            }
            match aliases.get(name.as_str()) {
                Some(aliased) => resolve_local_callable_fn_types(aliased, aliases, visited),
                None => Vec::new(),
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => intersection
            .types
            .iter()
            .flat_map(|member| resolve_local_callable_fn_types(member, aliases, visited))
            .collect(),
        _ => Vec::new(),
    }
}

/// Which of `.d.ts`'s two callable-shape AST nodes a `declare const`
/// export's own type annotation actually is. `Interface` is the shape
/// `extract_const_call_signature_decls`'s doc comment describes
/// (`SQLiteTableFn`-style, possibly several overloads, one
/// `TsCallSignatureDecl` per); `Direct` is a *self-contained* inline
/// function type with no interface involved at all -- real example: zod
/// v3's `lib/types.d.ts`, `declare const objectType: <T extends
/// ZodRawShape>(shape: T, params?: RawCreateParams) => ZodObject<...>;`,
/// later locally rename-exported (`export { objectType as object, ... }`)
/// -- the shape essentially all of zod v3's own primitives (`object`,
/// `string`, `number`, ...) use, unlike zod v4 (unaffected) or drizzle-
/// orm's interface-based `sqliteTable`. `TsCallSignatureDecl` and
/// `TsFnType` carry identical `params`/`type_params`/`type_ann` fields,
/// just with `type_ann` optional on one and required on the other --
/// kept as two small sibling `lower_dts_*` functions
/// (`lower_dts_call_signature`/`lower_dts_fn_type`) rather than one
/// shared generic helper, matching this file's existing convention of
/// one function per distinct AST shape.
enum CallableConstSignature<'a> {
    Interface(&'a TsCallSignatureDecl),
    Direct(&'a TsFnType),
}

/// Every top-level `declare const NAME: T;` (`let`/`var` too, exported or
/// bare -- a bare one only matters together with a later local rename-
/// export, see `all_reexported_function_declarations` in thaw-registry's
/// `install.rs`) whose type `T` is a callable shape: either a `TypeRef`
/// resolving (via `interfaces`, see `all_interface_decls_by_name`) to a
/// same-file interface with at least one call signature, or a direct
/// inline function type -- see `CallableConstSignature`'s own doc
/// comment for real examples of both. Parallel to `extract_fn_decls`/
/// `extract_interface_method_decls`: yields `NAME -> each callable
/// signature found (one per interface overload, or the one direct type),
/// so a caller can synthesize a `DtsFunction` per signature exactly like
/// an ordinary ambient function declaration.
fn extract_const_call_signature_decls<'a>(
    item: &'a ModuleItem,
    interfaces: &HashMap<String, &'a TsInterfaceDecl>,
    local_type_aliases: &HashMap<String, &'a TsType>,
) -> Vec<(String, CallableConstSignature<'a>)> {
    let var_decl = match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(var_decl))) => var_decl.as_ref(),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::Var(var_decl) => var_decl.as_ref(),
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    var_decl
        .decls
        .iter()
        .filter_map(|declarator| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            let annotation = binding.type_ann.as_ref()?;
            let name = binding.id.sym.to_string();
            // `A & { ... }` (`debug`: `debug.Debug & { debug: ...;
            // default: ... }`): a callable interface intersected with a
            // plain property bag. Use the callable member's signatures.
            let type_ann = match annotation.type_ann.as_ref() {
                TsType::TsUnionOrIntersectionType(
                    TsUnionOrIntersectionType::TsIntersectionType(intersection),
                ) => intersection
                    .types
                    .iter()
                    .map(Box::as_ref)
                    .find(|member| {
                        let TsType::TsTypeRef(ty_ref) = member else {
                            return matches!(
                                member,
                                TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(_))
                            );
                        };
                        let iface_name = match &ty_ref.type_name {
                            TsEntityName::Ident(ident) => ident.sym.to_string(),
                            TsEntityName::TsQualifiedName(qualified) => {
                                qualified.right.sym.to_string()
                            }
                        };
                        interfaces.get(iface_name.as_str()).is_some_and(|iface| {
                            iface.body.body.iter().any(|member| {
                                matches!(member, TsTypeElement::TsCallSignatureDecl(_))
                            })
                        })
                    })
                    .unwrap_or(annotation.type_ann.as_ref()),
                other => other,
            };
            match type_ann {
                TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
                    Some(vec![(name, CallableConstSignature::Direct(function))])
                }
                TsType::TsTypeRef(ty_ref) => {
                    let iface_name = match &ty_ref.type_name {
                        TsEntityName::Ident(ident) => ident.sym.to_string(),
                        TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
                    };
                    if let Some(iface) = interfaces.get(iface_name.as_str()) {
                        let signatures = iface
                            .body
                            .body
                            .iter()
                            .filter_map(|member| match member {
                                TsTypeElement::TsCallSignatureDecl(call) => {
                                    Some((name.clone(), CallableConstSignature::Interface(call)))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        if !signatures.is_empty() {
                            return Some(signatures);
                        }
                    }
                    // Not a same-file call-signature interface -- try
                    // resolving it as a (possibly intersected, possibly
                    // chained) local type alias instead. Real example:
                    // uuid's own `.d.ts` (via `@types/uuid`), `export
                    // const v4: v4;` where `type v4 = v4Buffer &
                    // v4String;` is a *local, unexported* alias, not an
                    // interface at all.
                    let functions = resolve_local_callable_fn_types(
                        annotation.type_ann.as_ref(),
                        local_type_aliases,
                        &mut HashSet::new(),
                    );
                    if functions.is_empty() {
                        return None;
                    }
                    Some(
                        functions
                            .into_iter()
                            .map(|function| {
                                (name.clone(), CallableConstSignature::Direct(function))
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            }
        })
        .flatten()
        .collect()
}

/// Like `lower_dts_method_signature`, for an interface's call signature
/// (`(...): T`) instead of a named method (`name(...): T`) -- the shape
/// `export declare const NAME: SomeCallableInterface;` uses, one
/// `TsCallSignatureDecl` per overload. `TsCallSignatureDecl` carries the
/// same `params`/`type_ann`/`type_params` fields as `TsMethodSignature`,
/// just without a `key`/`computed`/`optional` (a call signature has no
/// method name of its own -- the const's own binding name is used
/// instead). Kept as its own function rather than sharing code with
/// `lower_dts_method_signature`, matching this file's existing convention
/// of one small `lower_dts_*` function per distinct AST shape.
fn lower_dts_call_signature(
    name: &str,
    call: &TsCallSignatureDecl,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsFunction {
    let name = name.to_string();
    let generic = call.type_params.as_ref().map(|parameters| DtsGenericFunction {
        type_params: parameters
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.name.sym.to_string(),
                    parameter.constraint.as_deref().map(describe_ts_type),
                )
            })
            .collect(),
        param_types: call
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| describe_ts_type(&annotation.type_ann))
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_param_types: call
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        describe_generic_parameter_type(&annotation.type_ann, generic_interfaces)
                    })
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_rest_param_type: call.params.last().and_then(|parameter| {
            let TsFnParam::Rest(rest) = parameter else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: call
            .type_ann
            .as_ref()
            .map(|annotation| describe_ts_type(&annotation.type_ann))
            .unwrap_or_else(|| "JsValue".into()),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &call.type_params {
        for parameter in &type_params.params {
            let Some(constraint) = parameter
                .default
                .as_deref()
                .or(parameter.constraint.as_deref())
            else {
                continue;
            };
            if let DtsType::Native(constraint) = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            ) {
                substitution.insert(parameter.name.sym.to_string(), constraint);
            }
        }
    }
    let classify = |ty: &TsType| {
        resolve_ts_type_with_substitution(
            ty,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )
    };

    let rest_param = call.params.last().and_then(|param| {
        let TsFnParam::Rest(rest) = param else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => binding.id.sym.to_string(),
            _ => "rest".to_string(),
        };
        let ty = match rest.type_ann.as_ref() {
            Some(annotation) => match annotation.type_ann.as_ref() {
                TsType::TsArrayType(array) => classify(&array.elem_type),
                other => DtsType::Unsupported(format!(
                    "rest parameter must have an array type, found {}",
                    describe_ts_type(other)
                )),
            },
            None => DtsType::Unsupported("missing rest parameter type annotation".into()),
        };
        Some((name, ty))
    });
    let fixed_param_count = call.params.len() - usize::from(rest_param.is_some());
    let required_params = call
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| matches!(param, TsFnParam::Ident(binding) if !binding.id.optional))
        .count();
    let params = call
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let TsFnParam::Ident(binding) = param else {
                let reason =
                    "unsupported parameter pattern (only simple identifiers are classified yet)"
                        .to_string();
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = binding.id.sym.to_string();
            let ty = match &binding.type_ann {
                Some(ann) => classify(&ann.type_ann),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = match &call.type_ann {
        Some(ann) => classify(&ann.type_ann),
        None => DtsType::Native(HirType::Void),
    };

    DtsFunction {
        name,
        generic,
        params,
        required_params,
        rest_param,
        ret,
    }
}

/// Like `lower_dts_call_signature`, for a *direct* inline function type
/// (`(params) => Ret`) instead of an interface's call signature -- the
/// `CallableConstSignature::Direct` shape (see its own doc comment).
/// `TsFnType` carries the same `params`/`type_params` fields as
/// `TsCallSignatureDecl`, differing only in `type_ann`: always present
/// here (a function *type* is never written without one), rather than
/// optional. There can be only one of these per const (no interface-style
/// overloads), so unlike `extract_const_call_signature_decls`'s other
/// branch this never needs to be called more than once per name.
fn lower_dts_fn_type(
    name: &str,
    function: &TsFnType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsFunction {
    let name = name.to_string();
    let generic = function.type_params.as_ref().map(|parameters| DtsGenericFunction {
        type_params: parameters
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.name.sym.to_string(),
                    parameter.constraint.as_deref().map(describe_ts_type),
                )
            })
            .collect(),
        param_types: function
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| describe_ts_type(&annotation.type_ann))
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_param_types: function
            .params
            .iter()
            .map(|parameter| match parameter {
                TsFnParam::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        describe_generic_parameter_type(&annotation.type_ann, generic_interfaces)
                    })
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_rest_param_type: function.params.last().and_then(|parameter| {
            let TsFnParam::Rest(rest) = parameter else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: describe_ts_type(&function.type_ann.type_ann),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &function.type_params {
        for parameter in &type_params.params {
            let Some(constraint) = parameter
                .default
                .as_deref()
                .or(parameter.constraint.as_deref())
            else {
                continue;
            };
            if let DtsType::Native(constraint) = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            ) {
                substitution.insert(parameter.name.sym.to_string(), constraint);
            }
        }
    }
    let classify = |ty: &TsType| {
        resolve_ts_type_with_substitution(
            ty,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )
    };

    let rest_param = function.params.last().and_then(|param| {
        let TsFnParam::Rest(rest) = param else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => binding.id.sym.to_string(),
            _ => "rest".to_string(),
        };
        let ty = match rest.type_ann.as_ref() {
            Some(annotation) => match annotation.type_ann.as_ref() {
                TsType::TsArrayType(array) => classify(&array.elem_type),
                other => DtsType::Unsupported(format!(
                    "rest parameter must have an array type, found {}",
                    describe_ts_type(other)
                )),
            },
            None => DtsType::Unsupported("missing rest parameter type annotation".into()),
        };
        Some((name, ty))
    });
    let fixed_param_count = function.params.len() - usize::from(rest_param.is_some());
    let required_params = function
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| matches!(param, TsFnParam::Ident(binding) if !binding.id.optional))
        .count();
    let params = function
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let TsFnParam::Ident(binding) = param else {
                let reason =
                    "unsupported parameter pattern (only simple identifiers are classified yet)"
                        .to_string();
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = binding.id.sym.to_string();
            let ty = match &binding.type_ann {
                Some(ann) => classify(&ann.type_ann),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = classify(&function.type_ann.type_ann);

    DtsFunction {
        name,
        generic,
        params,
        required_params,
        rest_param,
        ret,
    }
}

/// For every top-level function declaration in `source` (see
/// `extract_fn_decls`) whose return type is a plain or namespace-
/// qualified type reference, `name -> <the reference's own bare
/// identifier>` -- independent of whether that reference ever resolves
/// to a `Native` `DtsType` at all (`classify_ts_type` still calls a
/// qualified name like `dayjs.Dayjs` `Unsupported`, since qualified
/// names aren't resolved). Lets a caller elsewhere (thaw-cli's
/// shims.rs) link a Fallback factory function's return value to one of
/// this same file's own classes (`parse_dts_classes`) without widening
/// `DtsFunction`'s own already very widely constructed shape for it.
/// Real example: dayjs's `declare function dayjs(...): dayjs.Dayjs`,
/// linking the `dayjs` factory function to its `Dayjs` class.
/// Every name a package's own `.d.ts` re-exports as a self-referential
/// namespace alias for its *own* already-flattened export table -- real
/// example: zod v4's own `index.d.cts`, `import * as z from "./v4/
/// classic/external.cjs"; export * from "./v4/classic/external.cjs";
/// export { z, z as default };`. `z` (and `default`) here don't name a
/// function, class, or interface at all -- they're bound purely by the
/// `import *`, so a plain `import { z } from "zod"` (as common in real
/// zod code as `import * as z from "zod"`, since both reach the exact
/// same object) has nothing in `parse_dts`'s own function table to
/// resolve `z` against, and fails outright ("`zod` has no export named
/// `z`") even though `import * as z from "zod"` -- a genuine namespace
/// import, needing no special per-name knowledge at all -- already
/// works. Returns every such alias name (here, `["z", "default"]`) so a
/// caller (thaw-cli's `shims.rs`) can mark them for the module graph to
/// treat a *named* import of one exactly like a namespace import: the
/// whole package's own export table, not a single symbol.
///
/// Deliberately narrow: only a bare `export { X[, X as Y] };` (no
/// `from` clause -- `X` must already be bound in this same file) whose
/// original name `X` is bound by a top-level `import * as X from
/// "...";` counts. A `declare namespace X { ... }` block re-exported
/// the same way is a structurally different (and still unsupported)
/// shape -- its members are declared *inside* it, not a star-import of
/// an already-flattened sibling module -- and isn't recognized here.
pub fn self_referential_namespace_aliases(source: &str) -> HashSet<String> {
    let Ok(module) = thaw_parser::parse_typescript(source) else {
        return HashSet::new();
    };
    let namespace_imports: HashSet<String> = module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
                return None;
            };
            Some(import.specifiers.iter().filter_map(|specifier| {
                match specifier {
                    swc_ecma_ast::ImportSpecifier::Namespace(namespace) => {
                        Some(namespace.local.sym.to_string())
                    }
                    _ => None,
                }
            }))
        })
        .flatten()
        .collect();
    let named_aliases = module.body.iter().filter_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = item else {
            return None;
        };
        if export.type_only || export.src.is_some() {
            return None;
        }
        Some(export.specifiers.iter().filter_map(|specifier| {
            let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else {
                return None;
            };
            if named.is_type_only {
                return None;
            }
            let export_name = |name: &swc_ecma_ast::ModuleExportName| match name {
                swc_ecma_ast::ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                swc_ecma_ast::ModuleExportName::Str(_) => None,
            };
            let original = export_name(&named.orig)?;
            if !namespace_imports.contains(&original) {
                return None;
            }
            Some(
                named
                    .exported
                    .as_ref()
                    .and_then(export_name)
                    .unwrap_or(original),
            )
        }))
    }).flatten();
    // `export default X;` (a *separate* AST node from `export { X as
    // default }` above, and the shape a real npm package commonly uses
    // instead -- real example: zod v3's `lib/index.d.ts`, `import * as z
    // from "./external"; export { z }; export default z;`) is equally a
    // self-referential alias of a namespace-imported name, just under the
    // implicit name `"default"`.
    let default_alias = module.body.iter().find_map(|item| {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) = item else {
            return None;
        };
        let Expr::Ident(ident) = default_expr.expr.as_ref() else {
            return None;
        };
        namespace_imports
            .contains(ident.sym.as_str())
            .then(|| "default".to_string())
    });
    named_aliases.chain(default_alias).collect()
}

/// Names explicitly exported only as TypeScript types. Registry imports of
/// these names are erased at runtime but still need a local type binding.
pub fn exported_type_names(source: &str) -> HashSet<String> {
    let Ok(module) = thaw_parser::parse_typescript(source) else {
        return HashSet::new();
    };
    module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => Some(export),
            _ => None,
        })
        .flat_map(|export| {
            export.specifiers.iter().filter_map(move |specifier| {
                let swc_ecma_ast::ExportSpecifier::Named(named) = specifier else {
                    return None;
                };
                if !export.type_only && !named.is_type_only {
                    return None;
                }
                let name = named.exported.as_ref().unwrap_or(&named.orig);
                match name {
                    swc_ecma_ast::ModuleExportName::Ident(ident) => Some(ident.sym.to_string()),
                    swc_ecma_ast::ModuleExportName::Str(string) => {
                        string.value.as_str().map(str::to_string)
                    }
                }
            })
        })
        .collect()
}

/// Every `declare namespace NAME { export { A as B, C as D, ... }; }`
/// block thaw-registry's own flattening emits for a real nested-
/// namespace re-export (`export * as NAME from "...";` -- e.g. zod's
/// `z.coerce`, `z.core`, `z.iso`; see thaw-registry's
/// `dts_source_with_reexported_functions`, whose doc comment this
/// mirrors). Maps `NAME -> { member name -> the real, already-flattened
/// top-level function name to call }`, so a caller (thaw-cli's
/// `shims.rs`/`module_graph.rs`) can rewrite a two-level member access
/// (`z.coerce.number(...)`) straight to the flattened function it
/// actually names.
///
/// Deliberately narrow, matching exactly what thaw-registry emits: a
/// bare (non-exported) `declare namespace NAME { ... }` whose entire
/// body is `export { ... };` specifiers with no `from` clause. A real
/// npm package's own hand-written `declare namespace X { function
/// foo(...): T; }` is a structurally different use of the same syntax
/// (already handled by `parse_dts`'s own namespace recursion, which
/// flattens straight to a bare name) and isn't recognized here.
pub fn nested_namespace_members(source: &str) -> HashMap<String, HashMap<String, String>> {
    use thaw_parser::ast::{ExportSpecifier, ModuleExportName, Stmt, TsModuleName};

    let Ok(module) = thaw_parser::parse_typescript(source) else {
        return HashMap::new();
    };
    module
        .body
        .iter()
        .filter_map(|item| {
            let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) = item else {
                return None;
            };
            let TsModuleName::Ident(name) = &module_decl.id else {
                return None;
            };
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return None;
            };
            let mut members = HashMap::new();
            for member in &block.body {
                let ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) = member else {
                    continue;
                };
                if export.type_only || export.src.is_some() {
                    continue;
                }
                for specifier in &export.specifiers {
                    let ExportSpecifier::Named(named) = specifier else {
                        continue;
                    };
                    if named.is_type_only {
                        continue;
                    }
                    let export_name = |name: &ModuleExportName| match name {
                        ModuleExportName::Ident(name) => Some(name.sym.to_string()),
                        ModuleExportName::Str(_) => None,
                    };
                    let Some(target) = export_name(&named.orig) else {
                        continue;
                    };
                    let Some(member_name) = named.exported.as_ref().and_then(export_name) else {
                        continue;
                    };
                    members.insert(member_name, target);
                }
            }
            if members.is_empty() {
                None
            } else {
                Some((name.sym.to_string(), members))
            }
        })
        .collect()
}

pub fn function_return_named_types(source: &str) -> HashMap<String, String> {
    let Ok(module) = thaw_parser::parse_typescript(source) else {
        return HashMap::new();
    };
    module
        .body
        .iter()
        .flat_map(extract_fn_decls)
        .filter_map(|(name, function)| {
            let ann = function.return_type.as_ref()?;
            let TsType::TsTypeRef(ty_ref) = ann.type_ann.as_ref() else {
                return None;
            };
            let type_name = match &ty_ref.type_name {
                TsEntityName::Ident(ident) => ident.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };
            Some((
                name.rsplit('.').next().unwrap_or(name).to_string(),
                type_name,
            ))
        })
        .collect()
}

