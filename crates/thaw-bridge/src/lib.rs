//! `.d.ts` -> Fast-path/Fallback classification. See
//! `docs/design/bridge.md` for the full design; this crate implements
//! sections 3 (parsing) and 4 (type classification) of it. `interface`
//! declarations are resolved into named object types (mirroring
//! `thaw_hir::lower::resolve_interfaces`; see `resolve_interfaces` below),
//! since `.d.ts` files lean on `interface` far more than inline `{ ... }`
//! type literals.
//!
//! This does *not* reuse `thaw_hir::lower_module`: a `.d.ts` file is a bag
//! of ambient signatures with no bodies at all and no `main`/`handler`
//! (which `lower_module` requires downstream), plus `.d.ts`-specific shapes
//! (`export`) that don't apply to regular `.ts` lowering. It does reuse
//! `thaw-parser` for the actual parse (see docs/design/bridge.md section 3
//! for why SWC, not tsc), and mirrors `thaw-hir::lower::lower_ts_type`'s
//! type-mapping rules by hand -- kept as a second implementation rather
//! than shared, since this one classifies into `Native`/`Unsupported`
//! instead of erroring, and the two crates' scopes (compiling one program
//! vs. classifying an arbitrary third-party signature) are different
//! enough that sharing code would mean threading a mode flag through
//! `lower_ts_type` for one caller.

use std::collections::HashMap;

use swc_ecma_ast::{
    Decl, Expr, FnDecl, Module, ModuleDecl, ModuleItem, Pat, TsEntityName, TsInterfaceDecl,
    TsKeywordTypeKind, TsType, TsTypeElement,
};
use thaw_hir::{FfiSignature, HirType};

/// One function signature extracted from a `.d.ts` file, before
/// classification.
#[derive(Debug, Clone, PartialEq)]
pub struct DtsFunction {
    pub name: String,
    pub params: Vec<(String, DtsType)>,
    pub ret: DtsType,
}

/// A parameter/return type as written in the `.d.ts`, before deciding
/// whether the whole signature is representable in Thaw's native type
/// system.
#[derive(Debug, Clone, PartialEq)]
pub enum DtsType {
    /// Maps cleanly onto a `thaw_hir::HirType`.
    Native(HirType),
    /// Doesn't map onto anything Thaw's native codegen supports today
    /// (generics, unions, callbacks, non-number array/object elements,
    /// unresolvable/self-referential interfaces, ...). Carries a
    /// human-readable reason.
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Classification {
    /// Every parameter and the return type mapped onto a native
    /// `HirType` -- compiles to a direct FFI call
    /// (`thaw_hir::HirExpr::FfiCall`), no QuickJS-NG involved.
    FastPath(FfiSignature),
    /// At least one parameter or the return type didn't map -- needs the
    /// QuickJS-NG fallback path (docs/design/bridge.md section 7, not
    /// implemented yet).
    Fallback { function: String, reason: String },
}

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
    module
        .body
        .iter()
        .filter_map(extract_fn_decl)
        .map(|fn_decl| lower_dts_function(fn_decl, &interfaces, &generic_interfaces))
        .collect()
}

fn extract_fn_decl(item: &ModuleItem) -> Option<&FnDecl> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Fn(fn_decl))) => Some(fn_decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::Fn(fn_decl) => Some(fn_decl),
            _ => None,
        },
        _ => None,
    }
}

fn extract_interface_decl(item: &ModuleItem) -> Option<&TsInterfaceDecl> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::TsInterface(iface))) => Some(iface),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::TsInterface(iface) => Some(iface),
            _ => None,
        },
        _ => None,
    }
}

type GenericInterfaces<'a> = HashMap<String, &'a TsInterfaceDecl>;

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
    let mut generic: GenericInterfaces = HashMap::new();
    for iface in module.body.iter().filter_map(extract_interface_decl) {
        let name = iface.id.sym.to_string();
        if iface.type_params.is_some() {
            generic.insert(name, iface);
        } else {
            raw.insert(name, iface);
        }
    }

    let mut resolved = HashMap::new();
    for name in raw.keys().cloned().collect::<Vec<_>>() {
        resolve_interface(&name, &raw, &mut resolved, &mut Vec::new());
    }
    (resolved, generic)
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<String, &TsInterfaceDecl>,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let Some(ty) = resolved.get(name) {
        return ty.clone();
    }
    if in_progress.iter().any(|n| n == name) {
        let ty = DtsType::Unsupported(format!("interface `{name}` is (indirectly) self-referential"));
        resolved.insert(name.to_string(), ty.clone());
        return ty;
    }
    let Some(iface) = raw.get(name) else {
        return DtsType::Unsupported(format!("unknown or generic interface `{name}`"));
    };

    in_progress.push(name.to_string());

    // `extends`: same rule as thaw-hir's `resolve_interface` -- base
    // fields first (in `extends`-list, then declaration, order), then this
    // interface's own fields; any name collision degrades the whole
    // interface to `Unsupported` rather than guessing an override rule.
    let mut fields: Vec<(String, HirType)> = Vec::new();
    let mut failure = None;
    'extends: for base in &iface.extends {
        if base.type_args.is_some() {
            failure = Some(
                "extends a base with type arguments, which is not classified yet".to_string(),
            );
            break;
        }
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            failure = Some("has an unsupported `extends` target (only a plain interface name)".to_string());
            break;
        };
        let base_name = base_ident.sym.to_string();
        match resolve_interface(&base_name, raw, resolved, in_progress) {
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
            DtsType::Native(_) => unreachable!("resolve_interface always returns an Object or Unsupported"),
            DtsType::Unsupported(reason) => {
                failure = Some(format!("extends unresolvable base `{base_name}`: {reason}"));
                break;
            }
        }
    }

    if failure.is_none() {
        for member in &iface.body.body {
            let TsTypeElement::TsPropertySignature(prop) = member else {
                failure = Some("has a non-property member (method/index signature)".to_string());
                break;
            };
            let field_name = match prop.key.as_ref() {
                Expr::Ident(ident) => ident.sym.to_string(),
                _ => {
                    failure = Some("has an unsupported property key".to_string());
                    break;
                }
            };
            if fields.iter().any(|(n, _)| *n == field_name) {
                failure = Some(format!(
                    "declares field `{field_name}`, which collides with an inherited field"
                ));
                break;
            }
            let field_ty = match &prop.type_ann {
                Some(ann) => resolve_type_with_interfaces(&ann.type_ann, raw, resolved, in_progress),
                None => DtsType::Unsupported(format!("field `{field_name}` has no type annotation")),
            };
            match field_ty {
                // Any type hir_codegen's `basic_type` can represent is fine
                // as a field now (fields are word-sized regardless of
                // their own type -- see hir_codegen.rs's `basic_type` for
                // the `HirType::Object` case), including a nested object.
                DtsType::Native(ty) => fields.push((field_name, ty)),
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
        None => DtsType::Native(HirType::Object(fields)),
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
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if raw.contains_key(ref_name) {
                return resolve_interface(ref_name, raw, resolved, in_progress);
            }
        }
    }
    // No generic interfaces here by design -- see `GenericInterfaces`'s
    // scope note.
    classify_ts_type(ty, resolved, &GenericInterfaces::new())
}

fn lower_dts_function(
    fn_decl: &FnDecl,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<DtsFunction, String> {
    let name = fn_decl.ident.sym.to_string();
    let func = &fn_decl.function;

    let params = func
        .params
        .iter()
        .map(|param| {
            let Pat::Ident(binding) = &param.pat else {
                return Err(format!(
                    "function `{name}` has an unsupported parameter pattern (only simple identifiers)"
                ));
            };
            let param_name = binding.id.sym.to_string();
            let ty = match &binding.type_ann {
                Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            Ok((param_name, ty))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let ret = match &func.return_type {
        Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
        None => DtsType::Native(HirType::Void),
    };

    Ok(DtsFunction { name, params, ret })
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
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => DtsType::Native(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => DtsType::Native(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => DtsType::Native(HirType::Bool),
            TsKeywordTypeKind::TsVoidKeyword => DtsType::Native(HirType::Void),
            other => DtsType::Unsupported(format!("unsupported keyword type {other:?}")),
        },

        TsType::TsArrayType(arr) => match classify_ts_type(&arr.elem_type, interfaces, generic_interfaces) {
            DtsType::Native(HirType::F64) => {
                DtsType::Native(HirType::Array(Box::new(HirType::F64)))
            }
            DtsType::Native(other) => DtsType::Unsupported(format!(
                "array element type {other:?} is not supported yet (only number[])"
            )),
            DtsType::Unsupported(reason) => {
                DtsType::Unsupported(format!("array element type: {reason}"))
            }
        },

        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            for member in &type_lit.members {
                let TsTypeElement::TsPropertySignature(prop) = member else {
                    return DtsType::Unsupported(
                        "object type literal has a non-property member (method/index signature)"
                            .to_string(),
                    );
                };
                let field_name = match prop.key.as_ref() {
                    Expr::Ident(ident) => ident.sym.to_string(),
                    _ => return DtsType::Unsupported("unsupported object type literal key".to_string()),
                };
                let field_ty = match &prop.type_ann {
                    Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
                    None => DtsType::Unsupported(format!("field `{field_name}` has no type annotation")),
                };
                match field_ty {
                    // See the parallel comment in `resolve_interface`:
                    // any representable type works as a field now.
                    DtsType::Native(ty) => fields.push((field_name, ty)),
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!("object field `{field_name}`: {reason}"))
                    }
                }
            }
            DtsType::Native(HirType::Object(fields))
        }

        TsType::TsTypeRef(ty_ref) => {
            let ref_name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(_) => {
                    return DtsType::Unsupported("qualified type names are not supported yet".to_string())
                }
            };

            // A name matching a resolved (non-generic) `interface` --
            // treated exactly like an inline `{ ... }` type literal.
            if let Some(resolved) = interfaces.get(&ref_name) {
                return resolved.clone();
            }
            // A generic interface, referenced with concrete type
            // arguments -- resolved on demand via substitution.
            if let Some(decl) = generic_interfaces.get(&ref_name) {
                return resolve_generic_interface(
                    &ref_name,
                    decl,
                    ty_ref,
                    interfaces,
                    generic_interfaces,
                    &mut Vec::new(),
                );
            }

            // Note: no `Array<T>`/`Promise<T>` recognition here (unlike
            // thaw-hir's `lower_ts_type`) -- a `.d.ts` signature using
            // either still needs a real decision about arena lifetime
            // (arrays) or the async ABI (promises) across a *foreign* FFI
            // boundary that section 5 of the design doc explicitly defers.
            DtsType::Unsupported(format!(
                "type reference `{ref_name}` is not classified yet (Array<T>/Promise<T>)"
            ))
        }

        other => DtsType::Unsupported(format!("unsupported type {other:?}")),
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
        return DtsType::Unsupported(format!("generic interface `{name}` cannot use `extends` yet"));
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
        match classify_ts_type(arg, interfaces, generic_interfaces) {
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
    let mut failure = None;
    for member in &decl.body.body {
        let TsTypeElement::TsPropertySignature(prop) = member else {
            failure = Some("has a non-property member (method/index signature)".to_string());
            break;
        };
        let field_name = match prop.key.as_ref() {
            Expr::Ident(ident) => ident.sym.to_string(),
            _ => {
                failure = Some("has an unsupported property key".to_string());
                break;
            }
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
            DtsType::Native(ty) => fields.push((field_name, ty)),
            DtsType::Unsupported(reason) => {
                failure = Some(format!("field `{field_name}`: {reason}"));
                break;
            }
        }
    }

    in_progress.pop();

    match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
        None => DtsType::Native(HirType::Object(fields)),
    }
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
            if let Some(decl) = generic_interfaces.get(ref_name) {
                return resolve_generic_interface(
                    ref_name,
                    decl,
                    ty_ref,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
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
                        ("Array", DtsType::Native(HirType::F64)) => {
                            return DtsType::Native(HirType::Array(Box::new(HirType::F64)))
                        }
                        ("Array", DtsType::Native(other)) => {
                            return DtsType::Unsupported(format!(
                                "array element type {other:?} is not supported yet (only number[])"
                            ))
                        }
                        ("Array", DtsType::Unsupported(reason)) => {
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
        TsType::TsArrayType(arr) => {
            match resolve_ts_type_with_substitution(
                &arr.elem_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(HirType::F64) => DtsType::Native(HirType::Array(Box::new(HirType::F64))),
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "array element type {other:?} is not supported yet (only number[])"
                )),
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("array element type: {reason}"))
                }
            }
        }
        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            for member in &type_lit.members {
                let TsTypeElement::TsPropertySignature(prop) = member else {
                    return DtsType::Unsupported(
                        "object type literal has a non-property member (method/index signature)"
                            .to_string(),
                    );
                };
                let field_name = match prop.key.as_ref() {
                    Expr::Ident(ident) => ident.sym.to_string(),
                    _ => return DtsType::Unsupported("unsupported object type literal key".to_string()),
                };
                let field_ty = match &prop.type_ann {
                    Some(ann) => resolve_ts_type_with_substitution(
                        &ann.type_ann,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    ),
                    None => DtsType::Unsupported(format!("field `{field_name}` has no type annotation")),
                };
                match field_ty {
                    DtsType::Native(ty) => fields.push((field_name, ty)),
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!("object field `{field_name}`: {reason}"))
                    }
                }
            }
            DtsType::Native(HirType::Object(fields))
        }
        other => classify_ts_type(other, interfaces, generic_interfaces),
    }
}

/// Classifies a whole function signature: `FastPath` only if *every*
/// parameter and the return type are `DtsType::Native` (see
/// docs/design/bridge.md section 4.2 for why partial native/dynamic
/// signatures aren't supported).
pub fn classify(func: &DtsFunction) -> Classification {
    let mut params = Vec::with_capacity(func.params.len());
    for (name, ty) in &func.params {
        match ty {
            DtsType::Native(hir_ty) => params.push(hir_ty.clone()),
            DtsType::Unsupported(reason) => {
                return Classification::Fallback {
                    function: func.name.clone(),
                    reason: format!("parameter `{name}`: {reason}"),
                }
            }
        }
    }

    let ret = match &func.ret {
        DtsType::Native(hir_ty) => hir_ty.clone(),
        DtsType::Unsupported(reason) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: format!("return type: {reason}"),
            }
        }
    };

    Classification::FastPath(FfiSignature {
        symbol: func.name.clone(),
        params,
        ret,
    })
}

/// Renders a `HirType` back into the TS syntax `thaw_hir::lower::lower_ts_type`
/// accepts, for `generate_shim`'s ambient declarations. Only ever called on
/// types that actually came from a successful classification (primitives,
/// `number[]`, and flat/nested objects), so the `Json`/`Union`/`Dynamic`
/// arms are just defensive completeness, not expected to be exercised.
fn render_ts_type(ty: &HirType) -> String {
    match ty {
        HirType::F64 | HirType::I64 => "number".to_string(),
        HirType::Bool => "boolean".to_string(),
        HirType::Void => "void".to_string(),
        HirType::Str => "string".to_string(),
        HirType::Json => "Json".to_string(),
        HirType::Array(elem) => format!("{}[]", render_ts_type(elem)),
        HirType::Promise(inner) => format!("Promise<{}>", render_ts_type(inner)),
        HirType::Object(fields) => {
            let rendered = fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", render_ts_type(ty)))
                .collect::<Vec<_>>()
                .join("; ");
            format!("{{ {rendered} }}")
        }
        HirType::Union(_) | HirType::Dynamic => "any".to_string(),
    }
}

/// Generates a Thaw-compilable TypeScript "shim" for a `.d.ts`'s functions,
/// per their classification (docs/design/bridge.md section 6/7): a
/// `FastPath` function becomes a plain ambient `declare function` (so
/// calling it compiles to a direct FFI call, resolved at link time --
/// still requires `thaw build --link <path>` today, until thaw-registry
/// can fetch/build that library automatically); a `Fallback` function
/// becomes a thin wrapper wired to the QuickJS-NG path (`callDynamic`).
///
/// This only generates the *callable surface* -- for `Fallback` functions,
/// something still has to `loadScript(...)` the package's actual JS source
/// before the wrapper is called (not generated here: there's no
/// module-level initialization mechanism in Thaw yet to hook that up
/// automatically, so it stays the caller's explicit responsibility, e.g.
/// as the first statement in `main`/`handler`).
pub fn generate_shim(functions: &[DtsFunction]) -> String {
    let mut out = String::new();
    for func in functions {
        match classify(func) {
            Classification::FastPath(sig) => {
                let params = func
                    .params
                    .iter()
                    .zip(&sig.params)
                    .map(|((name, _), ty)| format!("{name}: {}", render_ts_type(ty)))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "declare function {}({params}): {};\n",
                    sig.symbol,
                    render_ts_type(&sig.ret)
                ));
            }
            Classification::Fallback { function, reason } => {
                out.push_str(&format!("// Fallback (QuickJS-NG): {reason}\n"));
                // `argsArray` (not `args`): `callDynamic` expects a JSON
                // *array* of positional arguments, e.g.
                // `identity(JSON.parse("[42]"))`, not `identity(JSON.parse("42"))` --
                // named to make that convention hard to miss at the call site.
                out.push_str(&format!("function {function}(argsArray: Json): Json {{\n"));
                out.push_str(&format!("    return callDynamic(\"{function}\", argsArray);\n"));
                out.push_str("}\n");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_simple_primitive_signature_as_fast_path() {
        let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
        assert_eq!(funcs.len(), 1);
        assert_eq!(
            classify(&funcs[0]),
            Classification::FastPath(FfiSignature {
                symbol: "add".into(),
                params: vec![HirType::F64, HirType::F64],
                ret: HirType::F64,
            })
        );
    }

    #[test]
    fn classifies_number_array_and_flat_object_as_fast_path() {
        let funcs = parse_dts(
            "export declare function sum(xs: number[]): number;\n\
             export declare function dist(p: { x: number; y: number }): number;",
        )
        .unwrap();

        assert_eq!(
            classify(&funcs[0]),
            Classification::FastPath(FfiSignature {
                symbol: "sum".into(),
                params: vec![HirType::Array(Box::new(HirType::F64))],
                ret: HirType::F64,
            })
        );
        assert_eq!(
            classify(&funcs[1]),
            Classification::FastPath(FfiSignature {
                symbol: "dist".into(),
                params: vec![HirType::Object(vec![
                    ("x".into(), HirType::F64),
                    ("y".into(), HirType::F64),
                ])],
                ret: HirType::F64,
            })
        );
    }

    #[test]
    fn classifies_generic_function_as_fallback() {
        let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
    }

    #[test]
    fn classifies_union_parameter_as_fallback() {
        let funcs = parse_dts("export declare function f(x: string | number): void;").unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
    }

    #[test]
    fn classifies_string_array_as_fallback() {
        // Only number[] is supported today (Phase 1's own restriction).
        let funcs = parse_dts("export declare function f(xs: string[]): void;").unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
    }

    #[test]
    fn classifies_callback_parameter_as_fallback() {
        let funcs = parse_dts("export declare function f(cb: (err: string) => void): void;").unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
    }

    /// A plausible subset of a real package's `.d.ts` (uuid-shaped): mixes
    /// signatures that should and shouldn't classify as fast path.
    #[test]
    fn classifies_a_realistic_mixed_dts_file() {
        let source = r#"
            export declare function v4(): string;
            export declare function parse(input: string): number[];
            export declare function validate<T>(input: T): boolean;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 3);
        assert!(matches!(classify(&funcs[0]), Classification::FastPath(_)));
        assert!(matches!(classify(&funcs[1]), Classification::FastPath(_)));
        assert!(matches!(classify(&funcs[2]), Classification::Fallback { .. }));
    }

    #[test]
    fn classifies_interface_typed_signature_as_fast_path() {
        let source = r#"
            export interface Point {
                x: number;
                y: number;
            }
            export declare function dist(p: Point): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 1);
        assert_eq!(
            classify(&funcs[0]),
            Classification::FastPath(FfiSignature {
                symbol: "dist".into(),
                params: vec![HirType::Object(vec![
                    ("x".into(), HirType::F64),
                    ("y".into(), HirType::F64),
                ])],
                ret: HirType::F64,
            })
        );
    }

    #[test]
    fn interfaces_can_reference_each_other_regardless_of_order() {
        // `B` is declared before `A` and refers to it -- the resolver must
        // not depend on source order. Both interfaces are number-only, so
        // this still classifies as fast path (see the next test for what
        // happens when a field is itself a nested object).
        let source = r#"
            export interface B {
                a_sum: number;
                extra: number;
            }
            export interface A {
                x: number;
                y: number;
            }
            export declare function f(b: B): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::FastPath(_)));
    }

    #[test]
    fn nested_object_fields_classify_as_fast_path() {
        // `Line.start` is itself an object (`Point`), not a number --
        // hir_codegen's object layout now supports any representable
        // field type (fields are word-sized regardless of their own
        // type), including a nested object, so this classifies as fast
        // path just like a flat one.
        let source = r#"
            export interface Point {
                x: number;
                y: number;
            }
            export interface Line {
                start: Point;
                length: number;
            }
            export declare function len(l: Line): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(
            classify(&funcs[0]),
            Classification::FastPath(FfiSignature {
                symbol: "len".into(),
                params: vec![HirType::Object(vec![
                    (
                        "start".into(),
                        HirType::Object(vec![
                            ("x".into(), HirType::F64),
                            ("y".into(), HirType::F64),
                        ]),
                    ),
                    ("length".into(), HirType::F64),
                ])],
                ret: HirType::F64,
            })
        );
    }

    #[test]
    fn falls_back_on_self_referential_interface_without_breaking_other_functions() {
        let source = r#"
            export interface Node {
                value: number;
                next: Node;
            }
            export declare function head(n: Node): number;
            export declare function add(a: number, b: number): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
        assert!(matches!(classify(&funcs[1]), Classification::FastPath(_)));
    }

    #[test]
    fn classifies_generic_interface_instantiation_as_fast_path() {
        let source = r#"
            export interface Box<T> {
                value: T;
            }
            export declare function unwrap(b: Box<number>): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(
            classify(&funcs[0]),
            Classification::FastPath(FfiSignature {
                symbol: "unwrap".into(),
                params: vec![HirType::Object(vec![("value".into(), HirType::F64)])],
                ret: HirType::F64,
            })
        );
    }

    #[test]
    fn falls_back_on_wrong_number_of_generic_type_arguments() {
        let source = r#"
            export interface Pair<A, B> {
                first: A;
                second: B;
            }
            export declare function f(p: Pair<number>): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
    }

    #[test]
    fn falls_back_on_self_referential_generic_interface() {
        let source = r#"
            export interface Node<T> {
                value: T;
                next: Node<T>;
            }
            export declare function head(n: Node<number>): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
    }

    #[test]
    fn classifies_extends_as_fast_path_with_base_fields_prepended() {
        let source = r#"
            export interface Shape {
                color: number;
            }
            export interface Circle extends Shape {
                radius: number;
            }
            export declare function area(c: Circle): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(
            classify(&funcs[0]),
            Classification::FastPath(FfiSignature {
                symbol: "area".into(),
                params: vec![HirType::Object(vec![
                    ("color".into(), HirType::F64),
                    ("radius".into(), HirType::F64),
                ])],
                ret: HirType::F64,
            })
        );
    }

    #[test]
    fn falls_back_on_extends_field_collision() {
        let source = r#"
            export interface A { x: number; }
            export interface B extends A { x: number; }
            export declare function f(b: B): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert!(matches!(classify(&funcs[0]), Classification::Fallback { .. }));
    }

    #[test]
    fn generates_ambient_declaration_for_fast_path_function() {
        let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
        assert_eq!(
            generate_shim(&funcs),
            "declare function add(a: number, b: number): number;\n"
        );
    }

    #[test]
    fn generates_object_typed_ambient_declaration() {
        let funcs = parse_dts(
            r#"export interface Point { x: number; y: number; }
            export declare function dist(p: Point): number;"#,
        )
        .unwrap();
        assert_eq!(
            generate_shim(&funcs),
            "declare function dist(p: { x: number; y: number }): number;\n"
        );
    }

    #[test]
    fn generates_call_dynamic_wrapper_for_fallback_function() {
        let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
        let shim = generate_shim(&funcs);
        assert!(shim.contains("// Fallback (QuickJS-NG):"));
        assert!(shim.contains("function identity(argsArray: Json): Json {"));
        assert!(shim.contains(r#"return callDynamic("identity", argsArray);"#));
    }

    /// The generated shim isn't just plausible-looking text -- it must
    /// actually be valid, lowerable Thaw source. Compiles a realistic
    /// mixed `.d.ts` (fast path + fallback functions) into a shim, appends
    /// a `main` that calls both, and runs it through the real
    /// `thaw-parser`/`thaw-hir` pipeline used everywhere else.
    #[test]
    fn generated_shim_round_trips_through_real_lowering() {
        let dts = r#"
            export declare function add(a: number, b: number): number;
            export declare function identity<T>(x: T): T;
        "#;
        let funcs = parse_dts(dts).unwrap();
        let shim = generate_shim(&funcs);

        let program_source = format!(
            "{shim}\nfunction main(): void {{\n    console.log(add(2, 3));\n    const r = identity(JSON.parse(\"[1]\"));\n    console.log(Number(r));\n}}\n"
        );

        let module = thaw_parser::parse_typescript(&program_source)
            .unwrap_or_else(|e| panic!("generated shim did not parse: {e}\n---\n{program_source}"));
        let program = thaw_hir::lower_module(&module)
            .unwrap_or_else(|e| panic!("generated shim did not lower: {e}\n---\n{program_source}"));

        // `add` is ambient (fast path) -> extern_functions; `identity`'s
        // wrapper and `main` both have real bodies -> functions.
        assert_eq!(program.extern_functions.len(), 1);
        assert_eq!(program.extern_functions[0].symbol, "add");
        assert_eq!(program.functions.len(), 2);
        assert!(program.functions.iter().any(|f| f.name == "identity"));
        assert!(program.functions.iter().any(|f| f.name == "main"));
    }
}
