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
            match annotation.type_ann.as_ref() {
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

pub fn parse_dts_classes(source: &str) -> Result<Vec<DtsClass>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let mut classes = module
        .body
        .iter()
        .flat_map(extract_class_decls)
        .map(|(name, class)| lower_dts_class(name, class, &interfaces, &generic_interfaces))
        .collect::<Vec<_>>();
    let declared = classes
        .iter()
        .map(|class| (class.name.clone(), class.clone()))
        .collect::<HashMap<_, _>>();
    for class in &mut classes {
        if class.constructors.is_empty() {
            class.constructors = inherited_class_constructors(class, &declared, &mut Vec::new());
        }
        let (methods, properties) = inherited_class_members(class, &declared, &mut Vec::new());
        class.methods = methods;
        class.properties = properties;
    }
    for class in &mut classes {
        let constructor_overloaded = class.constructors.len() > 1;
        for constructor in &mut class.constructors {
            constructor.overloaded = constructor_overloaded;
        }
        let mut counts = HashMap::<(String, bool), usize>::new();
        for method in &class.methods {
            *counts
                .entry((method.name.clone(), method.is_static))
                .or_default() += 1;
        }
        for method in &mut class.methods {
            method.overloaded = counts[&(method.name.clone(), method.is_static)] > 1;
        }
    }
    Ok(classes)
}

fn inherited_class_constructors(
    class: &DtsClass,
    declared: &HashMap<String, DtsClass>,
    in_progress: &mut Vec<String>,
) -> Vec<DtsConstructor> {
    if !class.constructors.is_empty() || in_progress.contains(&class.name) {
        return class.constructors.clone();
    }
    in_progress.push(class.name.clone());
    let constructors = class
        .extends
        .as_ref()
        .and_then(|base| declared.get(base))
        .map(|base| inherited_class_constructors(base, declared, in_progress))
        .unwrap_or_default();
    in_progress.pop();
    constructors
}

fn inherited_class_members(
    class: &DtsClass,
    declared: &HashMap<String, DtsClass>,
    in_progress: &mut Vec<String>,
) -> (Vec<DtsMethod>, Vec<DtsProperty>) {
    if in_progress.contains(&class.name) {
        return (class.methods.clone(), class.properties.clone());
    }
    in_progress.push(class.name.clone());
    let (mut methods, mut properties) = class
        .extends
        .as_ref()
        .and_then(|base| declared.get(base))
        .map(|base| inherited_class_members(base, declared, in_progress))
        .unwrap_or_default();
    in_progress.pop();

    let shadows = |name: &str, is_static: bool| {
        class
            .methods
            .iter()
            .any(|member| member.name == name && member.is_static == is_static)
            || class
                .properties
                .iter()
                .any(|member| member.name == name && member.is_static == is_static)
    };
    methods.retain(|member| !shadows(&member.name, member.is_static));
    properties.retain(|member| !shadows(&member.name, member.is_static));
    methods.extend(class.methods.clone());
    properties.extend(class.properties.clone());
    (methods, properties)
}

/// Like `extract_fn_decls`/`extract_fn_decls_from_decl`, but for a class
/// declaration -- including one nested inside a `declare namespace X {
/// ... }` block, real-world example: dayjs's own `Dayjs` class, declared
/// inside `declare namespace dayjs { class Dayjs {...} } ` rather than
/// at the top level (its factory function `dayjs(...)`, by contrast, is
/// a genuinely top-level `declare function`). Without this, a
/// namespace-nested class was silently invisible to `parse_dts_classes`
/// entirely -- structurally parseable as a *type* (an ordinary
/// `TsTypeRef` resolves it via `resolve_interfaces` same as a top-level
/// one), but never bridgeable as a *class* since nothing here ever
/// extracted its own methods/constructors.
fn extract_class_decls(item: &ModuleItem) -> Vec<(&str, &Class)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_class_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_class_decls_from_decl(&export.decl)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Class(class) => class
                .ident
                .as_ref()
                .map(|ident| vec![(ident.sym.as_str(), class.class.as_ref())])
                .unwrap_or_default(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn extract_class_decls_from_decl(decl: &Decl) -> Vec<(&str, &Class)> {
    match decl {
        Decl::Class(class) => vec![(class.ident.sym.as_str(), &class.class)],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_class_decls).collect()
        }
        _ => Vec::new(),
    }
}

fn property_name(key: &PropName) -> Option<String> {
    match key {
        PropName::Ident(name) => Some(name.sym.to_string()),
        PropName::Str(name) => Some(name.value.to_string_lossy().into_owned()),
        PropName::Num(name) => Some(name.value.to_string()),
        _ => None,
    }
}

fn type_property_name(key: &Expr) -> Option<String> {
    match key {
        Expr::Ident(name) => Some(name.sym.to_string()),
        Expr::Lit(swc_ecma_ast::Lit::Str(name)) => {
            Some(name.value.to_string_lossy().into_owned())
        }
        Expr::Lit(swc_ecma_ast::Lit::Num(name)) => Some(name.value.to_string()),
        _ => None,
    }
}

fn index_signature_value(signature: &swc_ecma_ast::TsIndexSignature) -> Result<&TsType, String> {
    let [TsFnParam::Ident(key)] = signature.params.as_slice() else {
        return Err("index signature requires one identifier key".into());
    };
    let key_type = key
        .type_ann
        .as_ref()
        .ok_or("index signature key needs a type annotation")?;
    if !matches!(
        key_type.type_ann.as_ref(),
        TsType::TsKeywordType(keyword)
            if keyword.kind == TsKeywordTypeKind::TsStringKeyword
    ) {
        return Err("native dictionary index signatures require a string key".into());
    }
    signature
        .type_ann
        .as_ref()
        .map(|annotation| annotation.type_ann.as_ref())
        .ok_or_else(|| "index signature needs a value type annotation".into())
}

fn lower_class_params(
    params: &[ParamOrTsParamProp],
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Vec<(String, DtsType)> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let binding = match param {
                ParamOrTsParamProp::Param(param) => match &param.pat {
                    Pat::Ident(binding) => Some(binding),
                    Pat::Assign(assign) => match assign.left.as_ref() {
                        Pat::Ident(binding) => Some(binding),
                        _ => None,
                    },
                    _ => None,
                },
                ParamOrTsParamProp::TsParamProp(property) => match &property.param {
                    TsParamPropParam::Ident(binding) => Some(binding),
                    TsParamPropParam::Assign(assign) => match assign.left.as_ref() {
                        Pat::Ident(binding) => Some(binding),
                        _ => None,
                    },
                },
            };
            let Some(binding) = binding else {
                return (
                    format!("arg{index}"),
                    DtsType::Unsupported("unsupported constructor parameter pattern".into()),
                );
            };
            let ty = binding
                .type_ann
                .as_ref()
                .map(|annotation| {
                    classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                })
                .unwrap_or_else(|| DtsType::Unsupported("missing type annotation".into()));
            (binding.id.sym.to_string(), ty)
        })
        .collect()
}

fn class_param_is_required(param: &ParamOrTsParamProp) -> bool {
    match param {
        ParamOrTsParamProp::Param(param) => match &param.pat {
            Pat::Ident(binding) => !binding.optional,
            Pat::Assign(_) | Pat::Rest(_) => false,
            _ => true,
        },
        ParamOrTsParamProp::TsParamProp(property) => match &property.param {
            TsParamPropParam::Ident(binding) => !binding.optional,
            TsParamPropParam::Assign(_) => false,
        },
    }
}

fn is_public_member(accessibility: Option<Accessibility>) -> bool {
    accessibility.is_none_or(|accessibility| accessibility == Accessibility::Public)
}

fn hir_type_contains_callback(ty: &HirType) -> bool {
    match ty {
        HirType::Function(..) | HirType::CallableFunction(..) => true,
        HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner)
        | HirType::Array(inner) => hir_type_contains_callback(inner),
        HirType::Tuple(elements) | HirType::Union(elements) => {
            elements.iter().any(hir_type_contains_callback)
        }
        HirType::Object(fields) => fields
            .iter()
            .any(|(_, field)| hir_type_contains_callback(field)),
        _ => false,
    }
}

fn contextualize_native_callbacks(ty: HirType) -> HirType {
    match ty {
        HirType::CallableFunction(params, optional, rest, ret) if rest.is_none() => {
            let required = optional
                .first_at_or_after(0)
                .unwrap_or(params.len())
                .min(params.len());
            let params = params
                .into_iter()
                .map(|param| match param {
                    HirType::Optional(payload) => *payload,
                    other => other,
                })
                .collect::<Vec<_>>();
            HirType::Union(
                (required..=params.len())
                    .map(|arity| {
                        HirType::Function(
                            params[..arity].to_vec(),
                            Box::new(contextualize_native_callbacks(ret.as_ref().clone())),
                        )
                    })
                    .collect(),
            )
        }
        HirType::Object(fields) => HirType::Object(
            fields
                .into_iter()
                .map(|(name, ty)| (name, contextualize_native_callbacks(ty)))
                .collect(),
        ),
        HirType::Array(inner) => {
            HirType::Array(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Optional(inner) => {
            HirType::Optional(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Nullable(inner) => {
            HirType::Nullable(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Nullish(inner) => {
            HirType::Nullish(Box::new(contextualize_native_callbacks(*inner)))
        }
        HirType::Tuple(elements) => HirType::Tuple(
            elements
                .into_iter()
                .map(contextualize_native_callbacks)
                .collect(),
        ),
        HirType::Union(elements) => HirType::Union(
            elements
                .into_iter()
                .flat_map(|element| match contextualize_native_callbacks(element) {
                    HirType::Union(nested) => nested,
                    other => vec![other],
                })
                .collect(),
        ),
        other => other,
    }
}

/// Best-effort type used only to contextually type callbacks nested in a
/// fallback class method's dynamic object argument. Unknown callback values
/// stay as live `JsValue`s; unrelated object fields degrade to JSON.
fn contextual_dynamic_type(
    ty: &TsType,
    substitution: &HashMap<String, HirType>,
    interfaces: &HashMap<String, DtsType>,
    generic: &GenericInterfaces,
    dynamic_leaf: bool,
    in_progress: &mut Vec<String>,
) -> HirType {
    if let DtsType::Native(native) = resolve_ts_type_with_substitution(
        ty,
        substitution,
        interfaces,
        generic,
        &mut Vec::new(),
    ) {
        return contextualize_native_callbacks(native);
    }
    if dynamic_leaf {
        return HirType::JsValue;
    }
    match ty {
        TsType::TsParenthesizedType(value) => contextual_dynamic_type(
            &value.type_ann,
            substitution,
            interfaces,
            generic,
            false,
            in_progress,
        ),
        TsType::TsArrayType(array) => HirType::Array(Box::new(contextual_dynamic_type(
            &array.elem_type,
            substitution,
            interfaces,
            generic,
            false,
            in_progress,
        ))),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let DtsType::Native(result) = classify_native_union(union, |element| {
                DtsType::Native(contextual_dynamic_type(
                    element,
                    substitution,
                    interfaces,
                    generic,
                    false,
                    in_progress,
                ))
            }) else {
                unreachable!("contextual union classifier always returns native members")
            };
            result
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            let mut params = Vec::new();
            let mut optional = Vec::new();
            for parameter in &function.params {
                let TsFnParam::Ident(parameter) = parameter else {
                    continue;
                };
                if parameter.id.sym == "this" {
                    continue;
                }
                params.push(parameter.type_ann.as_ref().map_or(HirType::JsValue, |annotation| {
                    contextual_dynamic_type(
                        &annotation.type_ann,
                        substitution,
                        interfaces,
                        generic,
                        true,
                        in_progress,
                    )
                }));
                optional.push(parameter.id.optional);
            }
            let ret = contextual_dynamic_type(
                &function.type_ann.type_ann,
                substitution,
                interfaces,
                generic,
                true,
                in_progress,
            );
            if optional.iter().any(|optional| *optional) {
                let required = optional
                    .iter()
                    .position(|optional| *optional)
                    .unwrap_or(params.len());
                HirType::Union(
                    (required..=params.len())
                        .map(|arity| {
                            HirType::Function(params[..arity].to_vec(), Box::new(ret.clone()))
                        })
                        .collect(),
                )
            } else {
                HirType::Function(params, Box::new(ret))
            }
        }
        TsType::TsTypeRef(reference) => {
            let name = match &reference.type_name {
                TsEntityName::Ident(name) => name.sym.to_string(),
                TsEntityName::TsQualifiedName(name) => name.right.sym.to_string(),
            };
            if let Some(value) = substitution.get(&name) {
                return value.clone();
            }
            let declaration = if let Some(declaration) = generic.interfaces.get(&name) {
                declaration
            } else if let Some(alias) = generic.aliases.get(&name) {
                if in_progress.contains(&name) {
                    return HirType::Json;
                }
                in_progress.push(name.clone());
                let result = contextual_dynamic_type(
                    &alias.type_ann,
                    substitution,
                    interfaces,
                    generic,
                    false,
                    in_progress,
                );
                in_progress.pop();
                return result;
            } else {
                return HirType::Json;
            };
            if in_progress.contains(&name) {
                return HirType::Json;
            }
            let mut local = substitution.clone();
            if let Some(parameters) = &declaration.type_params {
                let arguments = reference
                    .type_params
                    .as_ref()
                    .map(|arguments| arguments.params.as_slice())
                    .unwrap_or_default();
                for (index, parameter) in parameters.params.iter().enumerate() {
                    let value = arguments
                        .get(index)
                        .map(|argument| {
                            contextual_dynamic_type(
                                argument,
                                substitution,
                                interfaces,
                                generic,
                                true,
                                in_progress,
                            )
                        })
                        .unwrap_or(HirType::JsValue);
                    local.insert(parameter.name.sym.to_string(), value);
                }
            }
            in_progress.push(name);
            let fields = declaration
                .body
                .body
                .iter()
                .filter_map(|member| {
                    let TsTypeElement::TsPropertySignature(property) = member else {
                        return None;
                    };
                    let name = type_property_name(&property.key)?;
                    let mut value = property.type_ann.as_ref().map_or(HirType::Json, |annotation| {
                        contextual_dynamic_type(
                            &annotation.type_ann,
                            &local,
                            interfaces,
                            generic,
                            false,
                            in_progress,
                        )
                    });
                    if property.optional {
                        value = optional_hir_type(value);
                    }
                    Some((name, value))
                })
                .collect();
            in_progress.pop();
            HirType::Object(fields)
        }
        _ => HirType::Json,
    }
}

fn lower_dts_class(
    name: &str,
    class: &Class,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsClass {
    let extends = class
        .super_class
        .as_deref()
        .and_then(|super_class| match super_class {
            Expr::Ident(name) => Some(name.sym.to_string()),
            _ => None,
        });
    let mut constructors = Vec::new();
    let mut methods = Vec::new();
    let mut properties = Vec::new();
    let has_constructor = class
        .body
        .iter()
        .any(|member| matches!(member, ClassMember::Constructor(_)));
    let constructible = !class.is_abstract
        && (!has_constructor
            || class.body.iter().any(|member| {
            matches!(
                member,
                ClassMember::Constructor(constructor)
                    if is_public_member(constructor.accessibility)
            )
        }));
    for member in &class.body {
        match member {
            ClassMember::Constructor(constructor)
                if is_public_member(constructor.accessibility) => constructors.push(DtsConstructor {
                params: lower_class_params(&constructor.params, interfaces, generic_interfaces),
                required_params: constructor
                    .params
                    .iter()
                    .take_while(|param| class_param_is_required(param))
                    .count(),
                overloaded: false,
            }),
            ClassMember::Method(method) if is_public_member(method.accessibility) => {
                let Some(method_name) = property_name(&method.key) else {
                    continue;
                };
                let function = lower_dts_function(
                    &method_name,
                    &method.function,
                    interfaces,
                    generic_interfaces,
                );
                let mut params = function.params;
                let mut substitution = HashMap::new();
                if let Some(parameters) = &method.function.type_params {
                    for parameter in &parameters.params {
                        substitution.insert(parameter.name.sym.to_string(), HirType::JsValue);
                    }
                }
                for (parameter, (_, classified)) in method.function.params.iter().zip(&mut params) {
                    if !matches!(classified, DtsType::Unsupported(_)) {
                        continue;
                    }
                    let Pat::Ident(parameter) = &parameter.pat else {
                        continue;
                    };
                    let Some(annotation) = &parameter.type_ann else {
                        continue;
                    };
                    let contextual = contextual_dynamic_type(
                        &annotation.type_ann,
                        &substitution,
                        interfaces,
                        generic_interfaces,
                        false,
                        &mut Vec::new(),
                    );
                    if hir_type_contains_callback(&contextual) {
                        *classified = DtsType::Native(contextual);
                    }
                }
                let rest_param = function.rest_param;
                methods.push(DtsMethod {
                    name: function.name,
                    params,
                    required_params: method
                        .function
                        .params
                        .iter()
                        .take_while(|param| match &param.pat {
                            Pat::Ident(binding) => !binding.optional,
                            Pat::Assign(_) | Pat::Rest(_) => false,
                            _ => true,
                        })
                        .count(),
                    rest_param,
                    ret: function.ret,
                    is_static: method.is_static,
                    kind: match method.kind {
                        MethodKind::Method => DtsMethodKind::Method,
                        MethodKind::Getter => DtsMethodKind::Getter,
                        MethodKind::Setter => DtsMethodKind::Setter,
                    },
                    overloaded: false,
                });
            }
            ClassMember::ClassProp(property) if is_public_member(property.accessibility) => {
                let Some(property_name) = property_name(&property.key) else {
                    continue;
                };
                if let Some(TsType::TsFnOrConstructorType(
                    TsFnOrConstructorType::TsFnType(function),
                )) = property.type_ann.as_ref().map(|annotation| annotation.type_ann.as_ref())
                {
                    let function = lower_dts_fn_type(
                        &property_name,
                        function,
                        interfaces,
                        generic_interfaces,
                    );
                    methods.push(DtsMethod {
                        name: function.name,
                        params: function.params,
                        required_params: function.required_params,
                        rest_param: function.rest_param,
                        ret: function.ret,
                        is_static: property.is_static,
                        kind: DtsMethodKind::Method,
                        overloaded: false,
                    });
                    continue;
                }
                let ty = property
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                    })
                    .unwrap_or_else(|| DtsType::Unsupported("missing type annotation".into()));
                let ty = match ty {
                    DtsType::Native(ty) if property.is_optional => {
                        DtsType::Native(optional_hir_type(ty))
                    }
                    other => other,
                };
                properties.push(DtsProperty {
                    name: property_name,
                    ty,
                    is_static: property.is_static,
                    readonly: property.readonly,
                });
            }
            _ => {}
        }
    }
    DtsClass {
        name: name.to_string(),
        extends,
        constructible,
        constructors,
        methods,
        properties,
    }
}

/// A single top-level `declare function`/`export declare function`
/// extracts one `(name, function)` pair; a `declare namespace Foo {
/// function bar(...): ...; }` recurses into its body and extracts every
/// function found inside (at any nesting depth -- a namespace can itself
/// contain a nested namespace). Found necessary by a real npm package
/// (`qs`), whose entire type surface -- including every function --
/// lives inside `declare namespace QueryString { ... }` rather than at
/// the top level; without this, `parse_dts` found zero functions in it.
/// The extracted `DtsFunction.name` is the bare function name (`parse`,
/// not `QueryString.parse`) -- that's also what `wrap_as_commonjs_module`'s
/// object-export hoisting binds it to at runtime (`qs`'s own
/// `module.exports = { parse, stringify, ... }`), so the two already
/// agree without any extra namespace-qualification logic.
///
/// Also handles a *named* `export default function foo(...): T;` (an
/// `ExportDefaultDecl`, a different AST shape than `ExportDecl` --
/// found necessary by real ESM packages, whose `.d.ts` commonly uses
/// this form, e.g. `escape-string-regexp`'s `export default function
/// escapeStringRegexp(string: string): string;`). An *anonymous*
/// `export default function(...): T;` has no name to extract a callable
/// `DtsFunction` under and is silently skipped -- `.d.ts` authors
/// essentially always name it in practice specifically so it's
/// referenceable, so this isn't expected to matter.
fn extract_fn_decls(item: &ModuleItem) -> Vec<(&str, &Function)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_fn_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_fn_decls_from_decl(&export.decl)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Fn(fn_expr) => match &fn_expr.ident {
                Some(ident) => vec![(ident.sym.as_str(), &fn_expr.function)],
                None => Vec::new(),
            },
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn extract_fn_decls_from_decl(decl: &Decl) -> Vec<(&str, &Function)> {
    match decl {
        Decl::Fn(fn_decl) => vec![(fn_decl.ident.sym.as_str(), &fn_decl.function)],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_fn_decls).collect()
        }
        _ => Vec::new(),
    }
}

/// The interface name that `export = <ident>;` together with `declare
/// const <ident>: <TypeRef>;` (`let`/`var` too) resolves to, if the file
/// uses that shape -- the specific pattern that identifies "this
/// interface IS the package's own namespace object" for
/// `extract_interface_method_decls`, as opposed to just some interface
/// used elsewhere as an ordinary parameter/return type (e.g. a callback
/// interface that has nothing to do with the package's own exports).
/// `<TypeRef>` may be namespace-qualified (`_.LoDashStatic`); only the
/// rightmost segment is used, matching how every interface here is
/// tracked by its own bare name regardless of which namespace it's
/// nested in.
fn export_assignment_interface_name(module: &Module) -> Option<String> {
    let exported = module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
            match export.expr.as_ref() {
                Expr::Ident(ident) => Some(ident.sym.to_string()),
                _ => None,
            }
        }
        _ => None,
    })?;
    module.body.iter().find_map(|item| {
        let ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Var(var_decl))) = item else {
            return None;
        };
        var_decl.decls.iter().find_map(|declarator| {
            let Pat::Ident(binding) = &declarator.name else {
                return None;
            };
            if binding.id.sym.as_str() != exported {
                return None;
            }
            let annotation = binding.type_ann.as_ref()?;
            let TsType::TsTypeRef(ty_ref) = annotation.type_ann.as_ref() else {
                return None;
            };
            match &ty_ref.type_name {
                TsEntityName::Ident(ident) => Some(ident.sym.to_string()),
                TsEntityName::TsQualifiedName(qualified) => {
                    Some(qualified.right.sym.to_string())
                }
            }
        })
    })
}

/// Every method signature from every declaration (top-level or inside a
/// `declare namespace` block) of the one interface named `target` --
/// parallel to `extract_fn_decls`, for an `export = obj` binding whose
/// declared type is an interface with method members rather than a
/// plain callable/namespace-function object. Real-world example:
/// lodash's `declare const _: _.LoDashStatic;` with `interface
/// LoDashStatic { chunk(...): ...; /* ~300 more */ }`, commonly re-opened
/// (TS declaration merging) across several files each augmenting the
/// same interface with a handful of methods -- rather than actually
/// merging same-named interfaces into one combined type, this just
/// extracts every declaration's own methods independently, the same
/// "grab every function-shaped thing, wherever it is" policy
/// `extract_fn_decls` already applies to plain functions; a name used by
/// more than one still only needs *a* signature that classifies, and
/// `Classification::classify_all` already falls back to `Fallback` for a
/// name with more than one anyway. Restricted to `target` (see
/// `export_assignment_interface_name`) rather than every interface in
/// the file: an interface used only as some other value's parameter or
/// return type (e.g. a callback interface with its own unrelated
/// methods) must not contribute spurious package-level functions.
fn extract_interface_method_decls<'a>(
    item: &'a ModuleItem,
    target: &str,
) -> Vec<(String, &'a TsMethodSignature)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => {
            extract_interface_method_decls_from_decl(decl, target)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_interface_method_decls_from_decl(&export.decl, target)
        }
        _ => Vec::new(),
    }
}

fn extract_interface_method_decls_from_decl<'a>(
    decl: &'a Decl,
    target: &str,
) -> Vec<(String, &'a TsMethodSignature)> {
    match decl {
        Decl::TsInterface(iface) if iface.id.sym.as_str() == target => iface
            .body
            .body
            .iter()
            .filter_map(|member| match member {
                TsTypeElement::TsMethodSignature(method) if !method.computed => {
                    type_property_name(&method.key).map(|name| (name, method))
                }
                _ => None,
            })
            .collect(),
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block
                .body
                .iter()
                .flat_map(|item| extract_interface_method_decls(item, target))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Like `extract_fn_decls`/`extract_class_decls`, but for an `interface`
/// declaration -- including one nested inside a `declare namespace X {
/// ... }` block, real-world example: `ms`'s own namespace carrying its
/// `Unit`/`UnitAnyCase`/`StringValue` helper types alongside its
/// `ms(...)` overloads. Without this, a namespace-nested interface (or,
/// via `extract_type_alias_decls`, type alias) was invisible to
/// `resolve_interfaces` entirely -- any reference to it anywhere, even a
/// same-namespace-qualified one (`ms.StringValue`), stayed permanently
/// `Unsupported`.
fn extract_interface_decls(item: &ModuleItem) -> Vec<&TsInterfaceDecl> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_interface_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_interface_decls_from_decl(&export.decl)
        }
        _ => Vec::new(),
    }
}

fn extract_interface_decls_from_decl(decl: &Decl) -> Vec<&TsInterfaceDecl> {
    match decl {
        Decl::TsInterface(iface) => vec![iface],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_interface_decls).collect()
        }
        _ => Vec::new(),
    }
}

fn extract_type_alias_decls(item: &ModuleItem) -> Vec<&swc_ecma_ast::TsTypeAliasDecl> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_type_alias_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_type_alias_decls_from_decl(&export.decl)
        }
        _ => Vec::new(),
    }
}

fn extract_type_alias_decls_from_decl(decl: &Decl) -> Vec<&swc_ecma_ast::TsTypeAliasDecl> {
    match decl {
        Decl::TsTypeAlias(alias) => vec![alias],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_type_alias_decls).collect()
        }
        _ => Vec::new(),
    }
}

#[derive(Default)]
struct GenericInterfaces<'a> {
    interfaces: HashMap<String, &'a TsInterfaceDecl>,
    aliases: HashMap<String, &'a swc_ecma_ast::TsTypeAliasDecl>,
}

/// Resolves every top-level *non-generic* `interface` into a `DtsType`
/// (first map), mirroring `thaw_hir::lower::resolve_interfaces` but
/// degrading to `DtsType::Unsupported` (with a reason) instead of erroring
/// on a self-referential/otherwise-unrepresentable interface -- one broken
/// interface should make signatures that use it fall back, not abort
/// classifying the rest of the `.d.ts` file. Generic interfaces are kept
/// raw (second map), resolved on demand via substitution -- see
/// `resolve_generic_interface`, and `thaw_hir::lower::GenericInterfaces`'s
/// doc comment for the scope limits this mirrors (no nested-inside-
/// another-interface use, no `extends` on the generic interface itself).
fn resolve_interfaces(module: &Module) -> (HashMap<String, DtsType>, GenericInterfaces<'_>) {
    let mut raw: HashMap<String, &TsInterfaceDecl> = HashMap::new();
    let mut names = Vec::new();
    let mut generic = GenericInterfaces::default();
    for iface in module.body.iter().flat_map(extract_interface_decls) {
        let name = iface.id.sym.to_string();
        if iface.type_params.is_some() {
            generic.interfaces.insert(name, iface);
        } else {
            if !raw.contains_key(&name) {
                names.push(name.clone());
            }
            raw.insert(name, iface);
        }
    }
    for alias in module
        .body
        .iter()
        .flat_map(extract_type_alias_decls)
        .filter(|alias| alias.type_params.is_some())
    {
        generic.aliases.insert(alias.id.sym.to_string(), alias);
    }

    let mut resolved = HashMap::new();
    for name in &names {
        resolve_interface(name, &raw, &generic, &mut resolved, &mut Vec::new());
    }
    let aliases = module
        .body
        .iter()
        .flat_map(extract_type_alias_decls)
        .filter(|alias| alias.type_params.is_none())
        .collect::<Vec<_>>();
    for _ in 0..raw.len() + aliases.len() {
        for name in &names {
            if matches!(resolved.get(name), Some(DtsType::Unsupported(_))) {
                resolved.remove(name);
            }
        }
        for name in &names {
            resolve_interface(name, &raw, &generic, &mut resolved, &mut Vec::new());
        }
        for alias in &aliases {
            let ty = classify_ts_type(&alias.type_ann, &resolved, &generic);
            resolved.insert(alias.id.sym.to_string(), ty);
        }
    }
    (resolved, generic)
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<String, &TsInterfaceDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let Some(ty) = resolved.get(name) {
        return ty.clone();
    }
    if in_progress.iter().any(|n| n == name) {
        let ty = DtsType::Unsupported(format!(
            "interface `{name}` is (indirectly) self-referential"
        ));
        resolved.insert(name.to_string(), ty.clone());
        return ty;
    }
    let Some(iface) = raw.get(name) else {
        return DtsType::Unsupported(format!("unknown or generic interface `{name}`"));
    };

    if iface
        .body
        .body
        .iter()
        .any(|member| matches!(member, TsTypeElement::TsCallSignatureDecl(_)))
    {
        let ty = DtsType::Native(HirType::JsValue);
        resolved.insert(name.to_string(), ty.clone());
        return ty;
    }

    in_progress.push(name.to_string());

    // `extends`: same rule as thaw-hir's `resolve_interface` -- base
    // fields first (in `extends`-list, then declaration, order), then this
    // interface's own fields; any name collision degrades the whole
    // interface to `Unsupported` rather than guessing an override rule.
    let mut fields: Vec<(String, HirType)> = Vec::new();
    let mut dictionary = None;
    let mut failure = None;
    'extends: for base in &iface.extends {
        if base.type_args.is_some() {
            failure =
                Some("extends a base with type arguments, which is not classified yet".to_string());
            break;
        }
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            failure = Some(
                "has an unsupported `extends` target (only a plain interface name)".to_string(),
            );
            break;
        };
        let base_name = base_ident.sym.to_string();
        match resolve_interface(&base_name, raw, generic, resolved, in_progress) {
            DtsType::Native(HirType::Object(base_fields)) => {
                for (field_name, field_ty) in base_fields {
                    if fields.iter().any(|(n, _)| *n == field_name) {
                        failure = Some(format!(
                            "inherits field `{field_name}` from `{base_name}`, which collides with an earlier field"
                        ));
                        break 'extends;
                    }
                    fields.push((field_name, field_ty));
                }
            }
            DtsType::Native(HirType::Dictionary(element)) => {
                if dictionary
                    .as_ref()
                    .is_some_and(|existing| existing != element.as_ref())
                    || fields.iter().any(|(_, field)| field != element.as_ref())
                {
                    failure = Some(format!(
                        "inherits an incompatible dictionary value from `{base_name}`"
                    ));
                    break;
                }
                dictionary = Some(*element);
            }
            DtsType::Native(_) => {
                unreachable!("resolve_interface always returns an Object or Unsupported")
            }
            DtsType::Unsupported(reason) => {
                failure = Some(format!("extends unresolvable base `{base_name}`: {reason}"));
                break;
            }
        }
    }

    if failure.is_none() {
        for member in &iface.body.body {
            if let TsTypeElement::TsIndexSignature(signature) = member {
                let value = match index_signature_value(signature) {
                    Ok(value) => resolve_type_with_interfaces(
                        value,
                        raw,
                        generic,
                        resolved,
                        in_progress,
                    ),
                    Err(reason) => DtsType::Unsupported(reason),
                };
                match value {
                    DtsType::Native(value)
                        if dictionary.as_ref().is_none_or(|existing| existing == &value)
                            && fields.iter().all(|(_, field)| field == &value) =>
                    {
                        dictionary = Some(value);
                    }
                    DtsType::Native(_) => {
                        failure = Some("declares an incompatible dictionary value type".into());
                        break;
                    }
                    DtsType::Unsupported(reason) => {
                        failure = Some(reason);
                        break;
                    }
                }
                continue;
            }
            let TsTypeElement::TsPropertySignature(prop) = member else {
                failure = Some("has a non-property member (method/index signature)".to_string());
                break;
            };
            let Some(field_name) = type_property_name(&prop.key) else {
                failure = Some("has an unsupported property key".to_string());
                break;
            };
            if fields.iter().any(|(n, _)| *n == field_name) {
                failure = Some(format!(
                    "declares field `{field_name}`, which collides with an inherited field"
                ));
                break;
            }
            let field_ty = match &prop.type_ann {
                Some(ann) => {
                    resolve_type_with_interfaces(
                        &ann.type_ann,
                        raw,
                        generic,
                        resolved,
                        in_progress,
                    )
                }
                None => {
                    DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                }
            };
            match field_ty {
                // Any type hir_codegen's `basic_type` can represent is fine
                // as a field now (fields are word-sized regardless of
                // their own type -- see hir_codegen.rs's `basic_type` for
                // the `HirType::Object` case), including a nested object.
                DtsType::Native(ty) => {
                    let ty = if prop.optional {
                        optional_hir_type(ty)
                    } else {
                        ty
                    };
                    if dictionary.as_ref().is_some_and(|element| element != &ty) {
                        failure = Some(format!(
                            "field `{field_name}` does not match its index value type"
                        ));
                        break;
                    }
                    fields.push((field_name, ty));
                }
                DtsType::Unsupported(reason) => {
                    failure = Some(format!("field `{field_name}`: {reason}"));
                    break;
                }
            }
        }
    }

    in_progress.pop();

    let result = match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
        None => DtsType::Native(dictionary.map_or(HirType::Object(fields), |element| {
            HirType::Dictionary(Box::new(element))
        })),
    };
    resolved.insert(name.to_string(), result.clone());
    result
}

/// Like `classify_ts_type`, but additionally resolves a `TsTypeRef` naming
/// a not-yet-resolved interface, recursively. Used only while building the
/// interface table (`resolve_interfaces`).
fn resolve_type_with_interfaces(
    ty: &TsType,
    raw: &HashMap<String, &TsInterfaceDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if raw.contains_key(ref_name) {
                return resolve_interface(ref_name, raw, generic, resolved, in_progress);
            }
        }
    }
    classify_ts_type(ty, resolved, generic)
}

/// Never fails: an unsupported parameter pattern (e.g. destructuring)
/// degrades that one parameter to `DtsType::Unsupported`, same as any
/// other unclassifiable type, rather than aborting this function -- and,
/// since `parse_dts` used to propagate that abort as a `Result::Err`
/// covering *every* function in the file, rather than aborting every
/// other function in the same `.d.ts` file along with it. Validated
/// against a real npm package's `.d.ts` corpus (date-fns): one function
/// with a destructured parameter (`function milliseconds({ years, ... }:
/// Duration)`) used to silently delete every other function in that file
/// from `parse_dts`'s result.
fn lower_dts_function(
    name: &str,
    func: &Function,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsFunction {
    let name = name.to_string();
    let generic = func.type_params.as_ref().map(|parameters| DtsGenericFunction {
        type_params: parameters
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.name.sym.to_string(),
                    parameter
                        .constraint
                        .as_deref()
                        .map(describe_ts_type),
                )
            })
            .collect(),
        param_types: func
            .params
            .iter()
            .map(|parameter| match &parameter.pat {
                Pat::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| describe_ts_type(&annotation.type_ann))
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_param_types: func
            .params
            .iter()
            .map(|parameter| match &parameter.pat {
                Pat::Ident(binding) => binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        describe_generic_parameter_type(&annotation.type_ann, generic_interfaces)
                    })
                    .unwrap_or_else(|| "Json".into()),
                _ => "Json".into(),
            })
            .collect(),
        contextual_rest_param_type: func.params.last().and_then(|parameter| {
            let Pat::Rest(rest) = &parameter.pat else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: func
            .return_type
            .as_ref()
            .map(|annotation| describe_ts_type(&annotation.type_ann))
            .unwrap_or_else(|| "JsValue".into()),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &func.type_params {
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

    let rest_param = func.params.last().and_then(|param| {
        let Pat::Rest(rest) = &param.pat else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => binding.id.sym.to_string(),
            _ => "rest".to_string(),
        };
        let ty = match rest.type_ann.as_ref() {
            Some(annotation) => match annotation.type_ann.as_ref() {
                TsType::TsArrayType(array) => {
                    classify(&array.elem_type)
                }
                other => DtsType::Unsupported(format!(
                    "rest parameter must have an array type, found {}",
                    describe_ts_type(other)
                )),
            },
            None => DtsType::Unsupported("missing rest parameter type annotation".into()),
        };
        Some((name, ty))
    });
    let fixed_param_count = func.params.len() - usize::from(rest_param.is_some());
    let required_params = func
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| matches!(&param.pat, Pat::Ident(binding) if !binding.optional))
        .count();
    let params = func
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let Pat::Ident(binding) = &param.pat else {
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

    let ret = match &func.return_type {
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

/// Like `lower_dts_function`, for an interface's method signature instead
/// of a plain ambient function declaration -- the same shape of
/// information (name, optional generics, fixed/rest parameters, return
/// type), just spread across `TsMethodSignature`'s own AST fields
/// (`TsFnParam` parameters, `type_ann` for the return) rather than
/// `Function`'s.
fn lower_dts_method_signature(
    name: &str,
    method: &TsMethodSignature,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsFunction {
    let name = name.to_string();
    let generic = method.type_params.as_ref().map(|parameters| DtsGenericFunction {
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
        param_types: method
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
        contextual_param_types: method
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
        contextual_rest_param_type: method.params.last().and_then(|parameter| {
            let TsFnParam::Rest(rest) = parameter else { return None };
            rest.type_ann.as_ref().map(|annotation| {
                let ty = rest_element_type(annotation.type_ann.as_ref());
                describe_contextual_rest_type(ty, generic_interfaces)
            })
        }),
        return_type: method
            .type_ann
            .as_ref()
            .map(|annotation| describe_ts_type(&annotation.type_ann))
            .unwrap_or_else(|| "JsValue".into()),
    });
    let mut substitution = HashMap::new();
    if let Some(type_params) = &method.type_params {
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

    let rest_param = method.params.last().and_then(|param| {
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
    let fixed_param_count = method.params.len() - usize::from(rest_param.is_some());
    let required_params = method
        .params
        .iter()
        .take(fixed_param_count)
        .take_while(|param| matches!(param, TsFnParam::Ident(binding) if !binding.optional))
        .count();
    let params = method
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

    let ret = match &method.type_ann {
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

/// Renders an arbitrary `.d.ts` type back to a short, TS-like expression,
/// for use in a `Classification::Fallback` `reason`. Validated against a
/// real npm package's `.d.ts` corpus (date-fns): without this, reasons for
/// anything beyond the simplest unsupported types were unreadable `{:?}`
/// dumps of the full AST node (spans, nested `Box`es, etc.) -- e.g.
/// `unsupported type TsUnionOrIntersectionType(TsIntersectionType(TsIntersectionType
/// { span: 1063..1081, types: [...] }))` for a type that's really just
/// `DateArg<Date> & {}`. This never needs to be exhaustive or fully
/// faithful (it's a diagnostic message, not something re-parsed), so
/// unusual type forms fall back to a short generic label instead of
/// recursing further.
fn describe_ts_type(ty: &TsType) -> String {
    match ty {
        TsType::TsKeywordType(kw) => keyword_name(kw.kind).to_string(),
        TsType::TsThisType(_) => "this".to_string(),
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            let params = function
                .params
                .iter()
                .enumerate()
                .map(|(index, parameter)| match parameter {
                    TsFnParam::Ident(parameter) => format!(
                        "{}{}: {}",
                        parameter.id.sym,
                        if parameter.id.optional { "?" } else { "" },
                        parameter
                            .type_ann
                            .as_ref()
                            .map(|annotation| describe_ts_type(&annotation.type_ann))
                            .unwrap_or_else(|| "Json".into())
                    ),
                    _ => format!("arg{index}: Json"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "({params}) => {}",
                describe_ts_type(&function.type_ann.type_ann)
            )
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(_)) => {
            "a constructor type".to_string()
        }
        TsType::TsTypeRef(ty_ref) => {
            let name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(_) => "<qualified name>".to_string(),
            };
            match &ty_ref.type_params {
                Some(params) => {
                    let args = params
                        .params
                        .iter()
                        .map(|p| describe_ts_type(p))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name}<{args}>")
                }
                None => name,
            }
        }
        TsType::TsTypeQuery(_) => "a `typeof` query type".to_string(),
        TsType::TsTypeLit(lit) if lit.members.is_empty() => "{}".to_string(),
        TsType::TsTypeLit(_) => "{ ... }".to_string(),
        TsType::TsArrayType(arr) => format!("{}[]", describe_ts_type(&arr.elem_type)),
        TsType::TsTupleType(_) => "a tuple type".to_string(),
        TsType::TsOptionalType(opt) => format!("{}?", describe_ts_type(&opt.type_ann)),
        TsType::TsRestType(rest) => format!("...{}", describe_ts_type(&rest.type_ann)),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(u)) => u
            .types
            .iter()
            .map(|t| describe_ts_type(t))
            .collect::<Vec<_>>()
            .join(" | "),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(i)) => i
            .types
            .iter()
            .map(|t| describe_ts_type(t))
            .collect::<Vec<_>>()
            .join(" & "),
        TsType::TsConditionalType(_) => "a conditional type".to_string(),
        TsType::TsInferType(_) => "an `infer` type".to_string(),
        TsType::TsParenthesizedType(p) => format!("({})", describe_ts_type(&p.type_ann)),
        TsType::TsTypeOperator(op) => {
            let op_name = match op.op {
                TsTypeOperatorOp::KeyOf => "keyof",
                TsTypeOperatorOp::Unique => "unique",
                TsTypeOperatorOp::ReadOnly => "readonly",
            };
            format!("{op_name} {}", describe_ts_type(&op.type_ann))
        }
        TsType::TsIndexedAccessType(indexed) => format!(
            "{}[{}]",
            describe_ts_type(&indexed.obj_type),
            describe_ts_type(&indexed.index_type)
        ),
        TsType::TsMappedType(_) => "a mapped type".to_string(),
        TsType::TsLitType(lit) => match &lit.lit {
            // `Wtf8Atom` has no `Display`; its `Debug` already renders as
            // a quoted string, which is exactly what's wanted here.
            TsLit::Str(s) => format!("{:?}", s.value),
            TsLit::Bool(b) => b.value.to_string(),
            TsLit::Number(n) => n.value.to_string(),
            _ => "a literal type".to_string(),
        },
        TsType::TsTypePredicate(_) => "a type predicate".to_string(),
        TsType::TsImportType(_) => "an `import()` type".to_string(),
    }
}

fn describe_generic_parameter_type(ty: &TsType, generic: &GenericInterfaces<'_>) -> String {
    let TsType::TsTypeRef(reference) = ty else {
        return describe_ts_type(ty);
    };
    let TsEntityName::Ident(name) = &reference.type_name else {
        return describe_ts_type(ty);
    };
    let Some(alias) = generic.aliases.get(name.sym.as_str()) else {
        return describe_ts_type(ty);
    };
    let Some(parameters) = &alias.type_params else {
        return describe_ts_type(ty);
    };
    let Some(arguments) = &reference.type_params else {
        return describe_ts_type(ty);
    };
    if parameters.params.len() != arguments.params.len() {
        return describe_ts_type(ty);
    }
    let substitutions = parameters
        .params
        .iter()
        .zip(&arguments.params)
        .map(|(parameter, argument)| {
            (
                parameter.name.sym.to_string(),
                describe_ts_type(argument),
            )
        })
        .collect::<HashMap<_, _>>();

    fn render(
        ty: &TsType,
        substitutions: &HashMap<String, String>,
        generic: &GenericInterfaces<'_>,
        depth: u8,
    ) -> String {
        match ty {
            TsType::TsTypeRef(reference) => {
                if let TsEntityName::Ident(name) = &reference.type_name {
                    if reference.type_params.is_none() {
                        if let Some(substitution) = substitutions.get(name.sym.as_str()) {
                            return substitution.clone();
                        }
                    }
                    if depth < 8 {
                        if let (Some(alias), Some(arguments)) = (
                            generic.aliases.get(name.sym.as_str()),
                            &reference.type_params,
                        ) {
                            if let Some(parameters) = &alias.type_params {
                                if parameters.params.len() == arguments.params.len() {
                                    let nested = parameters
                                        .params
                                        .iter()
                                        .zip(&arguments.params)
                                        .map(|(parameter, argument)| {
                                            (
                                                parameter.name.sym.to_string(),
                                                render(argument, substitutions, generic, depth + 1),
                                            )
                                        })
                                        .collect();
                                    return render(&alias.type_ann, &nested, generic, depth + 1);
                                }
                            }
                        }
                    }
                }
                describe_ts_type(ty)
            }
            TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
                let params = function
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, parameter)| match parameter {
                        TsFnParam::Ident(parameter) => format!(
                            "{}{}: {}",
                            parameter.id.sym,
                            if parameter.id.optional { "?" } else { "" },
                            parameter
                                .type_ann
                                .as_ref()
                                .map(|annotation| {
                                    render(&annotation.type_ann, substitutions, generic, depth)
                                })
                                .unwrap_or_else(|| "Json".into())
                        ),
                        _ => format!("arg{index}: Json"),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "({params}) => {}",
                    render(&function.type_ann.type_ann, substitutions, generic, depth)
                )
            }
            TsType::TsArrayType(array) => {
                format!("{}[]", render(&array.elem_type, substitutions, generic, depth))
            }
            TsType::TsIndexedAccessType(indexed) => format!(
                "{}[{}]",
                render(&indexed.obj_type, substitutions, generic, depth),
                render(&indexed.index_type, substitutions, generic, depth)
            ),
            TsType::TsTypeOperator(operator) => format!(
                "{} {}",
                match operator.op {
                    TsTypeOperatorOp::KeyOf => "keyof",
                    TsTypeOperatorOp::Unique => "unique",
                    TsTypeOperatorOp::ReadOnly => "readonly",
                },
                render(&operator.type_ann, substitutions, generic, depth)
            ),
            TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) =>
                union.types.iter().map(|ty| render(ty, substitutions, generic, depth)).collect::<Vec<_>>().join(" | "),
            TsType::TsParenthesizedType(parenthesized) => {
                format!("({})", render(&parenthesized.type_ann, substitutions, generic, depth))
            }
            _ => describe_ts_type(ty),
        }
    }

    render(&alias.type_ann, &substitutions, generic, 0)
}

fn rest_element_type(ty: &TsType) -> &TsType {
    match ty {
        TsType::TsArrayType(array) => &array.elem_type,
        TsType::TsTypeRef(reference)
            if matches!(&reference.type_name, TsEntityName::Ident(name) if name.sym == *"Array") =>
        {
            reference
                .type_params
                .as_ref()
                .and_then(|parameters| parameters.params.first())
                .map_or(ty, |element| element)
        }
        _ => ty,
    }
}

fn describe_contextual_rest_type(ty: &TsType, generic: &GenericInterfaces<'_>) -> String {
    let mut current = ty;
    for _ in 0..8 {
        let described = describe_generic_parameter_type(current, generic);
        if described.starts_with('(') {
            return described;
        }
        let TsType::TsTypeRef(reference) = current else {
            return described;
        };
        if let TsEntityName::Ident(name) = &reference.type_name {
            if let Some(alias) = generic.aliases.get(name.sym.as_str()) {
                if let TsType::TsUnionOrIntersectionType(
                    TsUnionOrIntersectionType::TsUnionType(union),
                ) = alias.type_ann.as_ref()
                {
                    if let Some(first) = union.types.first() {
                        let forwards_parameter = alias
                            .type_params
                            .as_ref()
                            .and_then(|parameters| parameters.params.first())
                            .is_some_and(|parameter| {
                                matches!(first.as_ref(), TsType::TsTypeRef(first_ref)
                                    if matches!(&first_ref.type_name, TsEntityName::Ident(first_name)
                                        if first_ref.type_params.is_none()
                                            && first_name.sym == parameter.name.sym))
                            });
                        if !forwards_parameter {
                            current = first;
                            continue;
                        }
                    }
                }
            }
        }
        let Some(next) = reference
            .type_params
            .as_ref()
            .and_then(|parameters| (parameters.params.len() == 1).then(|| &*parameters.params[0]))
        else {
            return described;
        };
        current = next;
    }
    describe_generic_parameter_type(current, generic)
}

/// The TS keyword spelling for a `TsKeywordTypeKind` (`number`/`string`/
/// `boolean`/`void` are handled natively by `classify_ts_type` and never
/// reach here; this covers the rest for `describe_ts_type`/error messages).
fn keyword_name(kind: TsKeywordTypeKind) -> &'static str {
    match kind {
        TsKeywordTypeKind::TsAnyKeyword => "any",
        TsKeywordTypeKind::TsUnknownKeyword => "unknown",
        TsKeywordTypeKind::TsNumberKeyword => "number",
        TsKeywordTypeKind::TsObjectKeyword => "object",
        TsKeywordTypeKind::TsBooleanKeyword => "boolean",
        TsKeywordTypeKind::TsBigIntKeyword => "bigint",
        TsKeywordTypeKind::TsStringKeyword => "string",
        TsKeywordTypeKind::TsSymbolKeyword => "symbol",
        TsKeywordTypeKind::TsVoidKeyword => "void",
        TsKeywordTypeKind::TsUndefinedKeyword => "undefined",
        TsKeywordTypeKind::TsNullKeyword => "null",
        TsKeywordTypeKind::TsNeverKeyword => "never",
        TsKeywordTypeKind::TsIntrinsicKeyword => "intrinsic",
    }
}

fn optional_hir_type(ty: HirType) -> HirType {
    match ty {
        HirType::Optional(_) | HirType::Nullish(_) => ty,
        HirType::Nullable(payload) => HirType::Nullish(payload),
        other => HirType::Optional(Box::new(other)),
    }
}

fn supports_native_array_element(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Str | HirType::Bool | HirType::Json | HirType::JsValue => true,
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            supports_native_array_element(payload)
        }
        HirType::Array(element) => supports_native_array_element(element),
        HirType::Tuple(elements) => elements.iter().all(supports_native_array_element),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supports_native_array_element(field)),
        _ => false,
    }
}

fn classify_indexed_access(object: DtsType, index: &TsType) -> DtsType {
    let index = match index {
        TsType::TsParenthesizedType(parenthesized) => parenthesized.type_ann.as_ref(),
        other => other,
    };
    let key = match index {
        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Str(value) => value.value.to_string_lossy().into_owned(),
            TsLit::Number(value) => value.value.to_string(),
            _ => {
                return DtsType::Unsupported(
                    "indexed access requires a string or number literal key".into(),
                )
            }
        },
        _ => {
            return DtsType::Unsupported(
                "indexed access requires one statically known property key".into(),
            )
        }
    };

    select_object_key(object, &key)
}

fn select_object_key(object: DtsType, key: &str) -> DtsType {
    match object {
        DtsType::Native(HirType::Object(fields)) => fields
            .into_iter()
            .find_map(|(name, ty)| (name == key).then_some(DtsType::Native(ty)))
            .unwrap_or_else(|| {
                DtsType::Unsupported(format!(
                    "indexed access key `{key}` does not exist on the object type"
                ))
            }),
        DtsType::Native(HirType::Dictionary(element)) => DtsType::Native(*element),
        DtsType::Native(other) => DtsType::Unsupported(format!(
            "indexed access requires an object type, found {other:?}"
        )),
        unsupported => unsupported,
    }
}

fn finite_utility_keys(ty: &TsType, keys: &mut Vec<String>) -> Result<(), String> {
    match ty {
        TsType::TsParenthesizedType(parenthesized) => {
            finite_utility_keys(&parenthesized.type_ann, keys)
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            for ty in &union.types {
                finite_utility_keys(ty, keys)?;
            }
            Ok(())
        }
        TsType::TsLitType(literal) => {
            let key = match &literal.lit {
                TsLit::Str(value) => value.value.to_string_lossy().into_owned(),
                TsLit::Number(value) => value.value.to_string(),
                _ => return Err("utility keys must be string or number literals".into()),
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
            Ok(())
        }
        _ => Err("utility keys must be a finite literal union".into()),
    }
}

fn utility_keys(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<String>, String> {
    if let TsType::TsTypeOperator(operator) = ty {
        if operator.op == TsTypeOperatorOp::KeyOf {
            return match classify_ts_type(&operator.type_ann, interfaces, generic_interfaces) {
                DtsType::Native(HirType::Object(fields)) => {
                    Ok(fields.into_iter().map(|(name, _)| name).collect())
                }
                DtsType::Native(_) => Err("keyof utility keys require an object type".into()),
                DtsType::Unsupported(reason) => Err(reason),
            };
        }
    }
    let mut keys = Vec::new();
    finite_utility_keys(ty, &mut keys)?;
    Ok(keys)
}

fn substituted_utility_keys(
    ty: &TsType,
    substitution: &HashMap<String, HirType>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    if let TsType::TsTypeOperator(operator) = ty {
        if operator.op == TsTypeOperatorOp::KeyOf {
            return match resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(HirType::Object(fields)) => {
                    Ok(fields.into_iter().map(|(name, _)| name).collect())
                }
                DtsType::Native(_) => Err("keyof utility keys require an object type".into()),
                DtsType::Unsupported(reason) => Err(reason),
            };
        }
    }
    let mut keys = Vec::new();
    finite_utility_keys(ty, &mut keys)?;
    Ok(keys)
}

fn apply_pick_or_omit(object: DtsType, keys: Result<Vec<String>, String>, omit: bool) -> DtsType {
    let keys = match keys {
        Ok(keys) => keys,
        Err(reason) => return DtsType::Unsupported(reason),
    };
    let DtsType::Native(HirType::Object(fields)) = object else {
        return DtsType::Unsupported("Pick/Omit require a fixed object type".into());
    };
    if let Some(key) = keys
        .iter()
        .find(|key| !fields.iter().any(|(name, _)| name == *key))
    {
        return DtsType::Unsupported(format!(
            "Pick/Omit key `{key}` does not exist on the object type"
        ));
    }
    DtsType::Native(HirType::Object(
        fields
            .into_iter()
            .filter(|(name, _)| keys.contains(name) != omit)
            .collect(),
    ))
}

fn apply_partial_or_required(object: DtsType, required: bool) -> DtsType {
    match object {
        DtsType::Native(HirType::Object(fields)) => DtsType::Native(HirType::Object(
            fields
                .into_iter()
                .map(|(name, ty)| {
                    let ty = if required {
                        match ty {
                            HirType::Optional(value) => *value,
                            HirType::Nullish(value) => HirType::Nullable(value),
                            other => other,
                        }
                    } else {
                        optional_hir_type(ty)
                    };
                    (name, ty)
                })
                .collect(),
        )),
        DtsType::Native(_) => DtsType::Unsupported(format!(
            "{}<T> requires a fixed object type",
            if required { "Required" } else { "Partial" }
        )),
        unsupported => unsupported,
    }
}

fn apply_record(value: DtsType, keys: Result<Vec<String>, String>) -> DtsType {
    let value = match value {
        DtsType::Native(value) => value,
        unsupported => return unsupported,
    };
    match keys {
        Ok(keys) => DtsType::Native(HirType::Object(
            keys.into_iter().map(|key| (key, value.clone())).collect(),
        )),
        Err(reason) => DtsType::Unsupported(reason),
    }
}

fn strip_non_nullable(ty: DtsType) -> DtsType {
    match ty {
        DtsType::Native(HirType::Optional(value))
        | DtsType::Native(HirType::Nullable(value))
        | DtsType::Native(HirType::Nullish(value)) => DtsType::Native(*value),
        other => other,
    }
}

fn classify_non_nullable_type(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsType {
    let TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) = ty
    else {
        return strip_non_nullable(classify_ts_type(
            ty,
            interfaces,
            generic_interfaces,
        ));
    };
    let mut native = None;
    for element in &union.types {
        if matches!(
            element.as_ref(),
            TsType::TsKeywordType(keyword)
                if matches!(
                    keyword.kind,
                    TsKeywordTypeKind::TsNullKeyword
                        | TsKeywordTypeKind::TsUndefinedKeyword
                )
        ) {
            continue;
        }
        match classify_ts_type(element, interfaces, generic_interfaces) {
            DtsType::Native(ty) if native.as_ref().is_none_or(|current| current == &ty) => {
                native = Some(ty)
            }
            DtsType::Native(_) => {
                return DtsType::Unsupported(
                    "NonNullable<T> has multiple incompatible native layouts".into(),
                )
            }
            unsupported => return unsupported,
        }
    }
    native
        .map(DtsType::Native)
        .unwrap_or_else(|| DtsType::Unsupported("NonNullable<T> has no native value".into()))
}

fn ts_type_includes_void(ty: &TsType) -> bool {
    match ty {
        TsType::TsKeywordType(keyword) => keyword.kind == TsKeywordTypeKind::TsVoidKeyword,
        TsType::TsParenthesizedType(parenthesized) => {
            ts_type_includes_void(&parenthesized.type_ann)
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            union.types.iter().any(|ty| ts_type_includes_void(ty))
        }
        _ => false,
    }
}

fn classify_native_union(
    union: &swc_ecma_ast::TsUnionType,
    mut classify: impl FnMut(&TsType) -> DtsType,
) -> DtsType {
    let mut native = Vec::new();
    let mut has_null = false;
    let mut has_undefined = false;
    for element in &union.types {
        match element.as_ref() {
            TsType::TsKeywordType(keyword) if keyword.kind == TsKeywordTypeKind::TsNullKeyword => {
                has_null = true;
            }
            TsType::TsKeywordType(keyword)
                if keyword.kind == TsKeywordTypeKind::TsUndefinedKeyword =>
            {
                has_undefined = true;
            }
            other => match classify(other) {
                DtsType::Native(HirType::Union(elements)) => {
                    for ty in elements {
                        if !native.contains(&ty) {
                            native.push(ty);
                        }
                    }
                }
                DtsType::Native(ty) if !native.contains(&ty) => native.push(ty),
                DtsType::Native(_) => {}
                unsupported => return unsupported,
            },
        }
    }
    if native.is_empty() {
        return DtsType::Unsupported("union has no native value type".into());
    }
    let payload = if native.len() == 1 {
        native.pop().unwrap()
    } else {
        HirType::Union(native)
    };
    let tagged = match (payload, has_null, has_undefined) {
        (HirType::Nullish(inner), _, _) => HirType::Nullish(inner),
        (HirType::Optional(inner), true, _) | (HirType::Nullable(inner), _, true) => {
            HirType::Nullish(inner)
        }
        (payload @ HirType::Optional(_), false, _)
        | (payload @ HirType::Nullable(_), _, false) => payload,
        (payload, true, true) => HirType::Nullish(Box::new(payload)),
        (payload, true, false) => HirType::Nullable(Box::new(payload)),
        (payload, false, true) => HirType::Optional(Box::new(payload)),
        (payload, false, false) => payload,
    };
    DtsType::Native(tagged)
}

fn remap_mapped_key(name_type: &TsType, parameter: &str, key: &str) -> Result<String, String> {
    match name_type {
        TsType::TsParenthesizedType(parenthesized) => {
            remap_mapped_key(&parenthesized.type_ann, parameter, key)
        }
        TsType::TsTypeRef(reference)
            if matches!(
                &reference.type_name,
                TsEntityName::Ident(name) if name.sym == parameter
            ) =>
        {
            Ok(key.to_string())
        }
        TsType::TsTypeRef(reference) => {
            let TsEntityName::Ident(name) = &reference.type_name else {
                return Err("qualified mapped key transforms are not supported".into());
            };
            let [argument] = reference
                .type_params
                .as_ref()
                .map(|params| params.params.as_slice())
                .unwrap_or_default()
            else {
                return Err("mapped key transform requires one type argument".into());
            };
            let value = remap_mapped_key(argument, parameter, key)?;
            match name.sym.as_str() {
                "Uppercase" => Ok(value.to_uppercase()),
                "Lowercase" => Ok(value.to_lowercase()),
                "Capitalize" => {
                    let mut chars = value.chars();
                    Ok(chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default())
                }
                "Uncapitalize" => {
                    let mut chars = value.chars();
                    Ok(chars
                        .next()
                        .map(|first| first.to_lowercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default())
                }
                _ => Err(format!("mapped key transform `{}` is not supported", name.sym)),
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let dynamic = intersection
                .types
                .iter()
                .filter(|ty| {
                    !matches!(
                        ty.as_ref(),
                        TsType::TsKeywordType(keyword)
                            if keyword.kind == TsKeywordTypeKind::TsStringKeyword
                    )
                })
                .collect::<Vec<_>>();
            match dynamic.as_slice() {
                [value] => remap_mapped_key(value, parameter, key),
                _ => Err("mapped key intersection is not statically resolvable".into()),
            }
        }
        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Str(value) => Ok(value.value.to_string_lossy().into_owned()),
            TsLit::Tpl(template) => {
                if template.quasis.len() != template.types.len() + 1 {
                    return Err("invalid mapped template literal key".into());
                }
                let mut rendered = String::new();
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let text = quasi
                        .cooked
                        .as_ref()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string());
                    rendered.push_str(&text);
                    if let Some(interpolation) = template.types.get(index) {
                        rendered.push_str(&remap_mapped_key(interpolation, parameter, key)?);
                    }
                }
                Ok(rendered)
            }
            _ => Err("mapped key remapping must produce a string key".into()),
        },
        _ => Err("mapped key remapping is not statically resolvable".into()),
    }
}

fn classify_mapped_type(
    mapped: &swc_ecma_ast::TsMappedType,
    outer_substitution: Option<&HashMap<String, HirType>>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    let Some(constraint) = &mapped.type_param.constraint else {
        return DtsType::Unsupported("mapped type key needs a finite constraint".into());
    };
    let keys = match outer_substitution {
        Some(substitution) => substituted_utility_keys(
            constraint,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        None => utility_keys(constraint, interfaces, generic_interfaces),
    };
    let keys = match keys {
        Ok(keys) => keys,
        Err(reason) => return DtsType::Unsupported(reason),
    };
    let Some(value_type) = &mapped.type_ann else {
        return DtsType::Unsupported("mapped type needs a value annotation".into());
    };
    let parameter = mapped.type_param.name.sym.as_str();
    let indexed_object = match value_type.as_ref() {
        TsType::TsIndexedAccessType(indexed)
            if matches!(
                indexed.index_type.as_ref(),
                TsType::TsTypeRef(reference)
                    if matches!(
                        &reference.type_name,
                        TsEntityName::Ident(name) if name.sym == parameter
                    )
            ) => Some(indexed.obj_type.as_ref()),
        _ => None,
    };

    let mut fields = Vec::with_capacity(keys.len());
    for key in keys {
        let value = if let Some(object) = indexed_object {
            let object = match outer_substitution {
                Some(substitution) => resolve_ts_type_with_substitution(
                    object,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ),
                None => classify_ts_type(object, interfaces, generic_interfaces),
            };
            select_object_key(object, &key)
        } else {
            let mut substitution = outer_substitution.cloned().unwrap_or_default();
            substitution.insert(parameter.to_string(), HirType::Str);
            resolve_ts_type_with_substitution(
                value_type,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        };
        let mut value = match value {
            DtsType::Native(value) => value,
            unsupported => return unsupported,
        };
        match mapped.optional {
            Some(TruePlusMinus::True | TruePlusMinus::Plus) => {
                value = optional_hir_type(value)
            }
            Some(TruePlusMinus::Minus) => {
                value = match value {
                    HirType::Optional(inner) => *inner,
                    HirType::Nullish(inner) => HirType::Nullable(inner),
                    other => other,
                }
            }
            None => {}
        }
        let field_name = match &mapped.name_type {
            Some(name_type) => match remap_mapped_key(name_type, parameter, &key) {
                Ok(name) => name,
                Err(reason) => return DtsType::Unsupported(reason),
            },
            None => key,
        };
        if fields.iter().any(|(name, _)| name == &field_name) {
            return DtsType::Unsupported(format!(
                "mapped key remapping produces duplicate field `{field_name}`"
            ));
        }
        fields.push((field_name, value));
    }
    DtsType::Native(HirType::Object(fields))
}

/// Mirrors `thaw_hir::lower::lower_ts_type`'s mapping rules, but never
/// fails: anything it can't map becomes `DtsType::Unsupported` with a
/// reason, for `classify` to report per-parameter/return instead of
/// aborting the whole `.d.ts` file over one unsupported signature.
fn classify_ts_type(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsType {
    match ty {
        // External instances are opaque handles today. Treat a fluent
        // `this` result as discarded so the method stays on its native
        // backend instead of mixing an N-API receiver into QuickJS.
        // ponytail: preserve the receiver once HIR has an external self type.
        TsType::TsThisType(_) => DtsType::Native(HirType::Void),
        TsType::TsParenthesizedType(parenthesized) => {
            classify_ts_type(&parenthesized.type_ann, interfaces, generic_interfaces)
        }
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::ReadOnly => {
            classify_ts_type(&operator.type_ann, interfaces, generic_interfaces)
        }
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::KeyOf => {
            match classify_ts_type(&operator.type_ann, interfaces, generic_interfaces) {
                DtsType::Native(HirType::Object(_) | HirType::Dictionary(_)) => {
                    DtsType::Native(HirType::Str)
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "keyof {other:?} does not have a string-only native ABI"
                )),
                unsupported => unsupported,
            }
        }
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => DtsType::Native(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => DtsType::Native(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => DtsType::Native(HirType::Bool),
            TsKeywordTypeKind::TsVoidKeyword => DtsType::Native(HirType::Void),
            other => DtsType::Unsupported(format!("`{}` is not supported", keyword_name(other))),
        },

        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Number(_) => DtsType::Native(HirType::F64),
            TsLit::Str(_) => DtsType::Native(HirType::Str),
            TsLit::Bool(_) => DtsType::Native(HirType::Bool),
            // A template literal type (`` `${number}${Unit}` ``) is,
            // structurally, always a plain string at runtime -- there's
            // no way to validate the actual interpolated pattern
            // without a real type checker, but erasing it to `Str` (the
            // same approach a plain string-literal type already gets)
            // is the same "best-effort classify, don't validate the
            // literal value" choice this whole match already makes.
            // Real example: `ms`'s own `StringValue = \`${number}\` |
            // \`${number}${UnitAnyCase}\` | ...`.
            TsLit::Tpl(_) => DtsType::Native(HirType::Str),
            _ => DtsType::Unsupported("unsupported literal type".into()),
        },

        TsType::TsConditionalType(conditional) => {
            let check = classify_ts_type(&conditional.check_type, interfaces, generic_interfaces);
            let extends =
                classify_ts_type(&conditional.extends_type, interfaces, generic_interfaces);
            match (check, extends) {
                (DtsType::Native(check), DtsType::Native(extends)) => classify_ts_type(
                    if bridge_type_satisfies_constraint(&check, &extends) {
                        &conditional.true_type
                    } else {
                        &conditional.false_type
                    },
                    interfaces,
                    generic_interfaces,
                ),
                (DtsType::Unsupported(reason), _) | (_, DtsType::Unsupported(reason)) => {
                    DtsType::Unsupported(format!("conditional type test: {reason}"))
                }
            }
        }

        TsType::TsIndexedAccessType(indexed) => classify_indexed_access(
            classify_ts_type(&indexed.obj_type, interfaces, generic_interfaces),
            &indexed.index_type,
        ),

        TsType::TsMappedType(mapped) => classify_mapped_type(
            mapped,
            None,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        ),

        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            classify_native_union(union, |element| {
                classify_ts_type(element, interfaces, generic_interfaces)
            })
        }

        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let mut native = Vec::new();
            for element in &intersection.types {
                match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(ty) => native.push(ty),
                    DtsType::Unsupported(_) => {
                        return DtsType::Unsupported(format!(
                            "unsupported type `{}`",
                            describe_ts_type(ty)
                        ))
                    }
                }
            }
            let Some(first) = native.first() else {
                return DtsType::Unsupported("empty intersection type is not supported".into());
            };
            if native.iter().all(|element| element == first) {
                return DtsType::Native(first.clone());
            }
            if native
                .iter()
                .all(|element| matches!(element, HirType::Object(_)))
            {
                let mut merged = Vec::new();
                for element in native {
                    let HirType::Object(fields) = element else {
                        unreachable!()
                    };
                    for (name, ty) in fields {
                        if let Some((_, existing)) =
                            merged.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &ty {
                                return DtsType::Unsupported(format!(
                                    "intersection field `{name}` has conflicting native layouts"
                                ));
                            }
                        } else {
                            merged.push((name, ty));
                        }
                    }
                }
                return DtsType::Native(HirType::Object(merged));
            }
            DtsType::Unsupported(format!("unsupported type `{}`", describe_ts_type(ty)))
        }

        TsType::TsArrayType(arr) => {
            match classify_ts_type(&arr.elem_type, interfaces, generic_interfaces) {
                DtsType::Native(element) if supports_native_array_element(&element) => {
                    DtsType::Native(HirType::Array(Box::new(element)))
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "array element type {other:?} does not have a native collection layout"
                )),
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("array element type: {reason}"))
                }
            }
        }

        TsType::TsTupleType(tuple) => {
            let elements = tuple
                .elem_types
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    match classify_ts_type(&element.ty, interfaces, generic_interfaces) {
                        DtsType::Native(element) => Ok(element),
                        DtsType::Unsupported(reason) => {
                            Err(format!("tuple element {index}: {reason}"))
                        }
                    }
                })
                .collect::<Result<Vec<_>, _>>();
            match elements {
                Ok(elements) => DtsType::Native(HirType::Tuple(elements)),
                Err(reason) => DtsType::Unsupported(reason),
            }
        }

        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                return DtsType::Unsupported("generic callback types are not supported".into());
            }
            let mut params = Vec::with_capacity(function.params.len());
            let mut optional = Vec::with_capacity(function.params.len());
            let mut rest = None;
            for (index, parameter) in function.params.iter().enumerate() {
                if matches!(parameter, TsFnParam::Ident(parameter) if parameter.id.sym == "this") {
                    continue;
                }
                let (annotation, is_optional) = match parameter {
                    TsFnParam::Ident(parameter) => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(format!(
                                "callback parameter `{}` has no type annotation",
                                parameter.id.sym
                            ));
                        };
                        (annotation, parameter.id.optional)
                    }
                    TsFnParam::Rest(parameter) if index + 1 == function.params.len() => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(
                                "callback rest parameter has no type annotation".into(),
                            );
                        };
                        let TsType::TsArrayType(array) = annotation.type_ann.as_ref() else {
                            return DtsType::Unsupported(
                                "callback rest parameter must use an array type".into(),
                            );
                        };
                        match classify_ts_type(&array.elem_type, interfaces, generic_interfaces) {
                            DtsType::Native(ty) => rest = Some(ty),
                            DtsType::Unsupported(_) => rest = Some(HirType::Json),
                        }
                        continue;
                    }
                    TsFnParam::Rest(_) => {
                        return DtsType::Unsupported("callback rest parameter must be last".into());
                    }
                    _ => {
                        return DtsType::Unsupported(
                            "callback parameters must be identifiers or a trailing rest parameter"
                                .into(),
                        );
                    }
                };
                match classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces) {
                    DtsType::Native(mut ty) => {
                        if is_optional {
                            ty = match ty {
                                HirType::Optional(_) | HirType::Nullish(_) => ty,
                                HirType::Nullable(payload) => HirType::Nullish(payload),
                                other => HirType::Optional(Box::new(other)),
                            };
                        }
                        params.push(ty);
                    }
                    // Callback values cross the JavaScript/N-API boundary as
                    // dynamic JSON. In real Node declarations the error slot
                    // is normally `Error | null` and result slots are often
                    // `any`; neither has a native AOT layout, but both have a
                    // faithful dynamic representation at this boundary.
                    DtsType::Unsupported(_) => params.push(HirType::Json),
                }
                optional.push(is_optional);
            }
            let ret = match classify_ts_type(
                &function.type_ann.type_ann,
                interfaces,
                generic_interfaces,
            ) {
                DtsType::Native(ret) => ret,
                DtsType::Unsupported(_) if ts_type_includes_void(&function.type_ann.type_ann) => {
                    HirType::Void
                }
                DtsType::Unsupported(reason) => {
                    return DtsType::Unsupported(format!("callback return type: {reason}"));
                }
            };
            DtsType::Native(if rest.is_some() || optional.iter().any(|value| *value) {
                let optional = HirOptionalMask::from_bools(&optional);
                HirType::CallableFunction(params, optional, rest.map(Box::new), Box::new(ret))
            } else {
                HirType::Function(params, Box::new(ret))
            })
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(_)) => {
            DtsType::Unsupported("constructor callback types are not supported".into())
        }

        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            let mut dictionary = None;
            for member in &type_lit.members {
                if let TsTypeElement::TsIndexSignature(signature) = member {
                    let value = match index_signature_value(signature) {
                        Ok(value) => {
                            classify_ts_type(value, interfaces, generic_interfaces)
                        }
                        Err(reason) => return DtsType::Unsupported(reason),
                    };
                    match value {
                        DtsType::Native(value)
                            if dictionary.as_ref().is_none_or(|existing| existing == &value)
                                && fields.iter().all(|(_, field)| field == &value) =>
                        {
                            dictionary = Some(value);
                        }
                        DtsType::Native(_) => {
                            return DtsType::Unsupported(
                                "object type literal has incompatible dictionary values".into(),
                            )
                        }
                        unsupported => return unsupported,
                    }
                    continue;
                }
                let TsTypeElement::TsPropertySignature(prop) = member else {
                    return DtsType::Unsupported(
                        "object type literal has a non-property member (method/index signature)"
                            .to_string(),
                    );
                };
                let Some(field_name) = type_property_name(&prop.key) else {
                    return DtsType::Unsupported(
                        "unsupported object type literal key".to_string(),
                    );
                };
                let field_ty = match &prop.type_ann {
                    Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
                    None => {
                        DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                    }
                };
                match field_ty {
                    // See the parallel comment in `resolve_interface`:
                    // any representable type works as a field now.
                    DtsType::Native(ty) => {
                        let ty = if prop.optional {
                            optional_hir_type(ty)
                        } else {
                            ty
                        };
                        if dictionary.as_ref().is_some_and(|element| element != &ty) {
                            return DtsType::Unsupported(format!(
                                "object field `{field_name}` does not match its index value type"
                            ));
                        }
                        fields.push((field_name, ty));
                    }
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}`: {reason}"
                        ))
                    }
                }
            }
            DtsType::Native(dictionary.map_or(HirType::Object(fields), |element| {
                HirType::Dictionary(Box::new(element))
            }))
        }

        TsType::TsTypeRef(ty_ref) => {
            // A qualified reference (`ms.StringValue`) is tracked by its
            // own bare rightmost identifier only, matching how every
            // interface/alias/class/function here already is regardless
            // of which namespace declares it (see
            // `export_assignment_interface_name`'s doc comment, and
            // `extract_interface_decls`/`extract_type_alias_decls`,
            // which populate `interfaces`/`generic_interfaces` this same
            // namespace-agnostic way) -- real example: `ms`'s own
            // `declare namespace ms { type StringValue = ...; }`,
            // referenced from its sibling overload as `ms.StringValue`.
            // Falls through to the ordinary "not classified" case below
            // exactly as before when no such name is known at all.
            let ref_name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(qualified) => qualified.right.sym.to_string(),
            };

            // A name matching a resolved (non-generic) `interface` --
            // treated exactly like an inline `{ ... }` type literal.
            if let Some(resolved) = interfaces.get(&ref_name) {
                return resolved.clone();
            }
            // A generic interface, referenced with concrete type
            // arguments -- resolved on demand via substitution.
            if let Some(decl) = generic_interfaces.interfaces.get(&ref_name) {
                return resolve_generic_interface(
                    &ref_name,
                    decl,
                    ty_ref,
                    None,
                    interfaces,
                    generic_interfaces,
                    &mut Vec::new(),
                );
            }
            if let Some(decl) = generic_interfaces.aliases.get(&ref_name) {
                return resolve_generic_alias(
                    &ref_name,
                    decl,
                    ty_ref,
                    None,
                    interfaces,
                    generic_interfaces,
                    &mut Vec::new(),
                );
            }
            if ref_name == "Json" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::Json);
            }
            if ref_name == "JsValue" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::JsValue);
            }
            // Mirrors `thaw_hir::lower::type_resolution`'s own `Date`
            // handling exactly: a fixed native object with a single
            // millisecond-since-epoch `timestamp` field, not a new value
            // representation. This lets a `.d.ts` `Date` parameter/return
            // (and, via constraint substitution above, a `<T extends
            // Date>` type parameter) classify as `Native` instead of
            // `Unsupported`, so ordinary Object JSON marshaling applies --
            // QuickJS-side `Date.prototype.toJSON`/the `JSON.parse`
            // reviver (thaw-quickjs) convert to/from a real JS `Date` at
            // the call boundary using this same `{"timestamp": ...}` shape.
            if ref_name == "Date" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::Object(vec![(
                    "timestamp".to_string(),
                    HirType::F64,
                )]));
            }
            if ref_name == "Readonly" {
                let Some(inner) = ty_ref
                    .type_params
                    .as_ref()
                    .and_then(|params| params.params.first())
                else {
                    return DtsType::Unsupported("Readonly<T> needs one type argument".to_string());
                };
                return classify_ts_type(inner, interfaces, generic_interfaces);
            }
            if matches!(ref_name.as_str(), "Partial" | "Required") {
                let [inner] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(format!(
                        "{ref_name}<T> requires exactly one type argument"
                    ));
                };
                return apply_partial_or_required(
                    classify_ts_type(inner, interfaces, generic_interfaces),
                    ref_name == "Required",
                );
            }
            if ref_name == "NonNullable" {
                let [inner] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(
                        "NonNullable<T> requires exactly one type argument".into(),
                    );
                };
                return classify_non_nullable_type(inner, interfaces, generic_interfaces);
            }
            if ref_name == "Record" {
                let [keys, value] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(
                        "Record<K, V> requires exactly two type arguments".into(),
                    );
                };
                let value = classify_ts_type(value, interfaces, generic_interfaces);
                if matches!(
                    keys.as_ref(),
                    TsType::TsKeywordType(keyword)
                        if keyword.kind == TsKeywordTypeKind::TsStringKeyword
                ) {
                    return match value {
                        DtsType::Native(value) => {
                            DtsType::Native(HirType::Dictionary(Box::new(value)))
                        }
                        unsupported => unsupported,
                    };
                }
                return apply_record(value, utility_keys(keys, interfaces, generic_interfaces));
            }
            if matches!(ref_name.as_str(), "Pick" | "Omit") {
                let [object, keys] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(format!(
                        "{ref_name}<T, K> requires exactly two type arguments"
                    ));
                };
                return apply_pick_or_omit(
                    classify_ts_type(object, interfaces, generic_interfaces),
                    utility_keys(keys, interfaces, generic_interfaces),
                    ref_name == "Omit",
                );
            }
            if matches!(ref_name.as_str(), "Array" | "ReadonlyArray") {
                let Some(element) = ty_ref
                    .type_params
                    .as_ref()
                    .and_then(|params| params.params.first())
                else {
                    return DtsType::Unsupported(format!(
                        "{ref_name}<T> needs one type argument"
                    ));
                };
                return match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(element) if supports_native_array_element(&element) => {
                        DtsType::Native(HirType::Array(Box::new(element)))
                    }
                    _ => DtsType::Unsupported("array element has no native collection layout".into()),
                };
            }

            // Note: no `Promise<T>` recognition here (unlike
            // thaw-hir's `lower_ts_type`) -- a `.d.ts` signature using
            // either still needs a real decision about arena lifetime
            // (arrays) or the async ABI (promises) across a *foreign* FFI
            // boundary that section 5 of the design doc explicitly defers.
            DtsType::Unsupported(format!(
                "type reference `{ref_name}` is not classified yet (Array<T>/Promise<T>)"
            ))
        }

        other => DtsType::Unsupported(format!("unsupported type `{}`", describe_ts_type(other))),
    }
}

/// Resolves `Name<ConcreteArg, ...>` for a generic interface `Name`,
/// mirroring `thaw_hir::lower::resolve_generic_interface` but degrading to
/// `DtsType::Unsupported` instead of erroring (wrong argument count,
/// self-reference, `extends`, or an unsupported member all degrade rather
/// than abort). Same scope limits as the thaw-hir version.
fn resolve_generic_interface(
    name: &str,
    decl: &TsInterfaceDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    outer_substitution: Option<&HashMap<String, HirType>>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if in_progress.iter().any(|n| n == name) {
        return DtsType::Unsupported(format!(
            "generic interface `{name}` is (indirectly) self-referential"
        ));
    }
    if !decl.extends.is_empty() {
        return DtsType::Unsupported(format!(
            "generic interface `{name}` cannot use `extends` yet"
        ));
    }

    let type_param_decl = decl
        .type_params
        .as_ref()
        .expect("caller only reaches here for a generic interface");
    let type_param_names: Vec<String> = type_param_decl
        .params
        .iter()
        .map(|p| p.name.sym.to_string())
        .collect();

    let type_args: &[Box<TsType>] = ty_ref
        .type_params
        .as_ref()
        .map(|params| params.params.as_slice())
        .unwrap_or(&[]);
    if type_args.len() != type_param_names.len() {
        return DtsType::Unsupported(format!(
            "interface `{name}` expects {} type argument(s), got {}",
            type_param_names.len(),
            type_args.len()
        ));
    }

    let mut substitution: HashMap<String, HirType> = HashMap::new();
    for (param_name, arg) in type_param_names.into_iter().zip(type_args) {
        let resolved = outer_substitution.map_or_else(
            || classify_ts_type(arg, interfaces, generic_interfaces),
            |outer| {
                resolve_ts_type_with_substitution(
                    arg,
                    outer,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )
            },
        );
        match resolved {
            DtsType::Native(ty) => {
                substitution.insert(param_name, ty);
            }
            DtsType::Unsupported(reason) => {
                return DtsType::Unsupported(format!("type argument for `{param_name}`: {reason}"))
            }
        }
    }

    in_progress.push(name.to_string());

    let mut fields = Vec::with_capacity(decl.body.body.len());
    let mut dictionary = None;
    let mut failure = None;
    for member in &decl.body.body {
        if let TsTypeElement::TsIndexSignature(signature) = member {
            let value = match index_signature_value(signature) {
                Ok(value) => resolve_ts_type_with_substitution(
                    value,
                    &substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ),
                Err(reason) => DtsType::Unsupported(reason),
            };
            match value {
                DtsType::Native(value)
                    if dictionary.as_ref().is_none_or(|existing| existing == &value)
                        && fields.iter().all(|(_, field)| field == &value) =>
                {
                    dictionary = Some(value);
                }
                DtsType::Native(_) => {
                    failure = Some("declares an incompatible dictionary value type".into());
                    break;
                }
                DtsType::Unsupported(reason) => {
                    failure = Some(reason);
                    break;
                }
            }
            continue;
        }
        let TsTypeElement::TsPropertySignature(prop) = member else {
            failure = Some("has a non-property member (method/index signature)".to_string());
            break;
        };
        let Some(field_name) = type_property_name(&prop.key) else {
            failure = Some("has an unsupported property key".to_string());
            break;
        };
        let field_ty = match &prop.type_ann {
            Some(ann) => resolve_ts_type_with_substitution(
                &ann.type_ann,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ),
            None => DtsType::Unsupported(format!("field `{field_name}` has no type annotation")),
        };
        match field_ty {
            DtsType::Native(ty) => {
                let ty = if prop.optional {
                    optional_hir_type(ty)
                } else {
                    ty
                };
                if dictionary.as_ref().is_some_and(|element| element != &ty) {
                    failure = Some(format!(
                        "field `{field_name}` does not match its index value type"
                    ));
                    break;
                }
                fields.push((field_name, ty));
            }
            DtsType::Unsupported(reason) => {
                failure = Some(format!("field `{field_name}`: {reason}"));
                break;
            }
        }
    }

    in_progress.pop();

    match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
        None => DtsType::Native(dictionary.map_or(HirType::Object(fields), |element| {
            HirType::Dictionary(Box::new(element))
        })),
    }
}

fn bridge_type_satisfies_constraint(actual: &HirType, constraint: &HirType) -> bool {
    if actual == constraint {
        return true;
    }
    match constraint {
        HirType::Union(elements) => elements
            .iter()
            .any(|element| bridge_type_satisfies_constraint(actual, element)),
        HirType::Object(required) => match actual {
            HirType::Object(fields) => required.iter().all(|(name, ty)| {
                fields
                    .iter()
                    .find(|(field, _)| field == name)
                    .is_some_and(|(_, actual)| bridge_type_satisfies_constraint(actual, ty))
            }),
            _ => false,
        },
        _ => false,
    }
}

fn resolve_generic_alias(
    name: &str,
    decl: &swc_ecma_ast::TsTypeAliasDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    outer_substitution: Option<&HashMap<String, HirType>>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if in_progress.iter().any(|active| active == name) {
        return DtsType::Unsupported(format!(
            "generic type alias `{name}` is (indirectly) self-referential"
        ));
    }
    let parameters = &decl
        .type_params
        .as_ref()
        .expect("caller only reaches generic aliases")
        .params;
    let arguments = ty_ref
        .type_params
        .as_ref()
        .map(|parameters| parameters.params.as_slice())
        .unwrap_or_default();
    let required = parameters
        .iter()
        .take_while(|parameter| parameter.default.is_none())
        .count();
    if arguments.len() < required || arguments.len() > parameters.len() {
        return DtsType::Unsupported(format!(
            "type alias `{name}` expects {}..={} type argument(s), got {}",
            required,
            parameters.len(),
            arguments.len()
        ));
    }

    let mut substitution = HashMap::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let concrete = if let Some(argument) = arguments.get(index) {
            match outer_substitution {
                Some(outer) => resolve_ts_type_with_substitution(
                    argument,
                    outer,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ),
                None => classify_ts_type(argument, interfaces, generic_interfaces),
            }
        } else {
            resolve_ts_type_with_substitution(
                parameter
                    .default
                    .as_ref()
                    .expect("arity validation requires a default"),
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        };
        let concrete = match concrete {
            DtsType::Native(concrete) => concrete,
            unsupported => return unsupported,
        };
        if let Some(constraint) = &parameter.constraint {
            let constraint = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            );
            let constraint = match constraint {
                DtsType::Native(constraint) => constraint,
                unsupported => return unsupported,
            };
            if !bridge_type_satisfies_constraint(&concrete, &constraint) {
                return DtsType::Unsupported(format!(
                    "type argument {concrete:?} does not satisfy constraint {constraint:?} for `{}` in alias `{name}`",
                    parameter.name.sym
                ));
            }
        }
        substitution.insert(parameter.name.sym.to_string(), concrete);
    }

    in_progress.push(name.to_string());
    let result = resolve_ts_type_with_substitution(
        &decl.type_ann,
        &substitution,
        interfaces,
        generic_interfaces,
        in_progress,
    );
    in_progress.pop();
    result
}

/// Like `classify_ts_type`, but a bare `TsTypeRef` matching one of `Name`'s
/// type parameters resolves to the corresponding concrete type instead of
/// an unknown-reference `Unsupported`. Mirrors
/// `thaw_hir::lower::resolve_ts_type_with_substitution`.
fn resolve_ts_type_with_substitution(
    ty: &TsType,
    substitution: &HashMap<String, HirType>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if let Some(concrete) = substitution.get(ref_name) {
                return DtsType::Native(concrete.clone());
            }
            if let Some(decl) = generic_interfaces.interfaces.get(ref_name) {
                return resolve_generic_interface(
                    ref_name,
                    decl,
                    ty_ref,
                    Some(substitution),
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
            }
            if let Some(decl) = generic_interfaces.aliases.get(ref_name) {
                return resolve_generic_alias(
                    ref_name,
                    decl,
                    ty_ref,
                    Some(substitution),
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
            }
            if ref_name == "Pick" || ref_name == "Omit" {
                let [object, keys] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(format!(
                        "{ref_name}<T, K> requires exactly two type arguments"
                    ));
                };
                let object = resolve_ts_type_with_substitution(
                    object,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                let keys = substituted_utility_keys(
                    keys,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                return apply_pick_or_omit(object, keys, ref_name == "Omit");
            }
            if ref_name == "Record" {
                let [keys, value] = ty_ref
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default()
                else {
                    return DtsType::Unsupported(
                        "Record<K, V> requires exactly two type arguments".into(),
                    );
                };
                let value = resolve_ts_type_with_substitution(
                    value,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                if matches!(
                    keys.as_ref(),
                    TsType::TsKeywordType(keyword)
                        if keyword.kind == TsKeywordTypeKind::TsStringKeyword
                ) {
                    return match value {
                        DtsType::Native(value) => {
                            DtsType::Native(HirType::Dictionary(Box::new(value)))
                        }
                        unsupported => unsupported,
                    };
                }
                let keys = substituted_utility_keys(
                    keys,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                return apply_record(value, keys);
            }
            if let Some(params) = &ty_ref.type_params {
                if let [elem] = params.params.as_slice() {
                    let resolved_elem = resolve_ts_type_with_substitution(
                        elem,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    );
                    match (ref_name, resolved_elem) {
                        ("Readonly", resolved) => return resolved,
                        ("Partial", resolved) => {
                            return apply_partial_or_required(resolved, false)
                        }
                        ("Required", resolved) => {
                            return apply_partial_or_required(resolved, true)
                        }
                        ("NonNullable", resolved) => return strip_non_nullable(resolved),
                        ("Array" | "ReadonlyArray", DtsType::Native(element))
                            if supports_native_array_element(&element) => {
                            return DtsType::Native(HirType::Array(Box::new(element)))
                        }
                        ("Array" | "ReadonlyArray", DtsType::Native(other)) => {
                            return DtsType::Unsupported(format!(
                                "array element type {other:?} does not have a native collection layout"
                            ))
                        }
                        ("Array" | "ReadonlyArray", DtsType::Unsupported(reason)) => {
                            return DtsType::Unsupported(format!("array element type: {reason}"))
                        }
                        _ => {}
                    }
                }
            }
        }
        return classify_ts_type(ty, interfaces, generic_interfaces);
    }

    match ty {
        TsType::TsParenthesizedType(parenthesized) => resolve_ts_type_with_substitution(
            &parenthesized.type_ann,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::ReadOnly => {
            resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        }
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::KeyOf => {
            match resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(HirType::Object(_) | HirType::Dictionary(_)) => {
                    DtsType::Native(HirType::Str)
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "keyof {other:?} does not have a string-only native ABI"
                )),
                unsupported => unsupported,
            }
        }
        TsType::TsConditionalType(conditional) => {
            let check = resolve_ts_type_with_substitution(
                &conditional.check_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            );
            let extends = resolve_ts_type_with_substitution(
                &conditional.extends_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            );
            match (check, extends) {
                (DtsType::Native(check), DtsType::Native(extends)) => {
                    resolve_ts_type_with_substitution(
                        if bridge_type_satisfies_constraint(&check, &extends) {
                            &conditional.true_type
                        } else {
                            &conditional.false_type
                        },
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )
                }
                (DtsType::Unsupported(reason), _) | (_, DtsType::Unsupported(reason)) => {
                    DtsType::Unsupported(format!("conditional type test: {reason}"))
                }
            }
        }
        TsType::TsIndexedAccessType(indexed) => classify_indexed_access(
            resolve_ts_type_with_substitution(
                &indexed.obj_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ),
            &indexed.index_type,
        ),
        TsType::TsMappedType(mapped) => classify_mapped_type(
            mapped,
            Some(substitution),
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            classify_native_union(union, |element| {
                resolve_ts_type_with_substitution(
                    element,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )
            })
        }
        TsType::TsArrayType(arr) => {
            match resolve_ts_type_with_substitution(
                &arr.elem_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(element) if supports_native_array_element(&element) => {
                    DtsType::Native(HirType::Array(Box::new(element)))
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "array element type {other:?} does not have a native collection layout"
                )),
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("array element type: {reason}"))
                }
            }
        }
        TsType::TsTupleType(tuple) => {
            let elements = tuple
                .elem_types
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    match resolve_ts_type_with_substitution(
                        &element.ty,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    ) {
                        DtsType::Native(element) => Ok(element),
                        DtsType::Unsupported(reason) => {
                            Err(format!("tuple element {index}: {reason}"))
                        }
                    }
                })
                .collect::<Result<Vec<_>, _>>();
            match elements {
                Ok(elements) => DtsType::Native(HirType::Tuple(elements)),
                Err(reason) => DtsType::Unsupported(reason),
            }
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                return DtsType::Unsupported("generic callback types are not supported".into());
            }
            let mut params = Vec::with_capacity(function.params.len());
            let mut optional = Vec::with_capacity(function.params.len());
            let mut rest = None;
            for (index, parameter) in function.params.iter().enumerate() {
                if matches!(parameter, TsFnParam::Ident(parameter) if parameter.id.sym == "this") {
                    continue;
                }
                let (annotation, is_optional) = match parameter {
                    TsFnParam::Ident(parameter) => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(format!(
                                "callback parameter `{}` has no type annotation",
                                parameter.id.sym
                            ));
                        };
                        (annotation.type_ann.as_ref(), parameter.id.optional)
                    }
                    TsFnParam::Rest(parameter) if index + 1 == function.params.len() => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(
                                "callback rest parameter has no type annotation".into(),
                            );
                        };
                        let TsType::TsArrayType(array) = annotation.type_ann.as_ref() else {
                            return DtsType::Unsupported(
                                "callback rest parameter must use an array type".into(),
                            );
                        };
                        match resolve_ts_type_with_substitution(
                            &array.elem_type,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        ) {
                            DtsType::Native(ty) => rest = Some(ty),
                            DtsType::Unsupported(_) => rest = Some(HirType::Json),
                        }
                        continue;
                    }
                    TsFnParam::Rest(_) => {
                        return DtsType::Unsupported("callback rest parameter must be last".into());
                    }
                    _ => {
                        return DtsType::Unsupported(
                            "callback parameters must be identifiers or a trailing rest parameter"
                                .into(),
                        );
                    }
                };
                let mut ty = match resolve_ts_type_with_substitution(
                    annotation,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                ) {
                    DtsType::Native(ty) => ty,
                    DtsType::Unsupported(_) => HirType::Json,
                };
                if is_optional {
                    ty = optional_hir_type(ty);
                }
                params.push(ty);
                optional.push(is_optional);
            }
            let ret = match resolve_ts_type_with_substitution(
                &function.type_ann.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(ret) => ret,
                DtsType::Unsupported(_) if ts_type_includes_void(&function.type_ann.type_ann) => {
                    HirType::Void
                }
                DtsType::Unsupported(reason) => {
                    return DtsType::Unsupported(format!("callback return type: {reason}"));
                }
            };
            DtsType::Native(if rest.is_some() || optional.iter().any(|value| *value) {
                HirType::CallableFunction(
                    params,
                    HirOptionalMask::from_bools(&optional),
                    rest.map(Box::new),
                    Box::new(ret),
                )
            } else {
                HirType::Function(params, Box::new(ret))
            })
        }
        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            let mut dictionary = None;
            for member in &type_lit.members {
                if let TsTypeElement::TsIndexSignature(signature) = member {
                    let value = match index_signature_value(signature) {
                        Ok(value) => resolve_ts_type_with_substitution(
                            value,
                            substitution,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        ),
                        Err(reason) => return DtsType::Unsupported(reason),
                    };
                    match value {
                        DtsType::Native(value)
                            if dictionary.as_ref().is_none_or(|existing| existing == &value)
                                && fields.iter().all(|(_, field)| field == &value) =>
                        {
                            dictionary = Some(value);
                        }
                        DtsType::Native(_) => {
                            return DtsType::Unsupported(
                                "object type literal has incompatible dictionary values".into(),
                            )
                        }
                        unsupported => return unsupported,
                    }
                    continue;
                }
                let TsTypeElement::TsPropertySignature(prop) = member else {
                    return DtsType::Unsupported(
                        "object type literal has a non-property member (method/index signature)"
                            .to_string(),
                    );
                };
                let Some(field_name) = type_property_name(&prop.key) else {
                    return DtsType::Unsupported(
                        "unsupported object type literal key".to_string(),
                    );
                };
                let field_ty = match &prop.type_ann {
                    Some(ann) => resolve_ts_type_with_substitution(
                        &ann.type_ann,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    ),
                    None => {
                        DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                    }
                };
                match field_ty {
                    DtsType::Native(ty) => {
                        let ty = if prop.optional {
                            optional_hir_type(ty)
                        } else {
                            ty
                        };
                        if dictionary.as_ref().is_some_and(|element| element != &ty) {
                            return DtsType::Unsupported(format!(
                                "object field `{field_name}` does not match its index value type"
                            ));
                        }
                        fields.push((field_name, ty));
                    }
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}`: {reason}"
                        ))
                    }
                }
            }
            DtsType::Native(dictionary.map_or(HirType::Object(fields), |element| {
                HirType::Dictionary(Box::new(element))
            }))
        }
        other => classify_ts_type(other, interfaces, generic_interfaces),
    }
}
