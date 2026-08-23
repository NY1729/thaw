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
    Decl, DefaultDecl, Expr, Function, Module, ModuleDecl, ModuleItem, Pat, TsEntityName,
    TsInterfaceDecl, TsKeywordTypeKind, TsLit, TsNamespaceBody, TsType, TsTypeElement,
    TsTypeOperatorOp, TsUnionOrIntersectionType,
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
    Ok(module
        .body
        .iter()
        .flat_map(extract_fn_decls)
        .map(|(name, func)| lower_dts_function(name, func, &interfaces, &generic_interfaces))
        .collect())
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

    let params = func
        .params
        .iter()
        .enumerate()
        .map(|(i, param)| {
            let Pat::Ident(binding) = &param.pat else {
                let reason = "unsupported parameter pattern (only simple identifiers are classified yet)".to_string();
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = binding.id.sym.to_string();
            let ty = match &binding.type_ann {
                Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = match &func.return_type {
        Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
        None => DtsType::Native(HirType::Void),
    };

    DtsFunction { name, params, ret }
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
        TsType::TsFnOrConstructorType(_) => "a function type".to_string(),
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
        TsType::TsIndexedAccessType(_) => "an indexed access type".to_string(),
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
            other => DtsType::Unsupported(format!("`{}` is not supported", keyword_name(other))),
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

/// Classifies every function in `functions`, but only once per distinct
/// name: a `.d.ts` overload set (multiple `declare function foo(...)`
/// signatures sharing a name -- common in real npm packages, e.g. `ms`'s
/// `ms(value: number, options?): string` / `ms(value: string): number`)
/// can't become a single native `extern "C"` symbol the way Fast path
/// needs, even when one individual overload would classify Fast path on
/// its own. So a name with more than one signature always falls back as
/// a whole; Fallback's `callDynamic` doesn't care about a fixed shape,
/// so the real JS function is free to handle whatever overload logic it
/// wants. A name with exactly one signature classifies exactly as
/// `classify` would.
///
/// Found necessary by running a real overloaded package (`ms`) through
/// `generate_shim`: without this, an overload set whose members classify
/// differently produced two separate, name-colliding top-level
/// declarations, and thaw-hir/codegen silently picked whichever was
/// declared last -- no error, and the outcome depended entirely on
/// declaration order rather than being a real decision.
pub fn classify_all(functions: &[DtsFunction]) -> Vec<(String, Classification)> {
    let mut by_name: Vec<(&str, Vec<&DtsFunction>)> = Vec::new();
    for func in functions {
        match by_name.iter_mut().find(|(name, _)| *name == func.name) {
            Some((_, group)) => group.push(func),
            None => by_name.push((&func.name, vec![func])),
        }
    }

    by_name
        .into_iter()
        .map(|(name, group)| {
            let classification = match group.as_slice() {
                [only] => classify(only),
                overloads => Classification::Fallback {
                    function: name.to_string(),
                    reason: format!(
                        "`{name}` has {} overloaded signatures in the .d.ts; Fast path needs exactly one",
                        overloads.len()
                    ),
                },
            };
            (name.to_string(), classification)
        })
        .collect()
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
/// `classify_all`, then downgrades every FastPath entry to Fallback when
/// `native_lib_available` is false: a real npm package fetched via
/// `thaw registry add` is pure JS with no native counterpart at all, so a
/// type shape that happens to be fully representable in a native ABI
/// (e.g. date-fns's `daysToWeeks(days: number): number`) still has
/// nothing to link against. Treating it as FastPath anyway produces a
/// confusing `undefined reference` linker error for a function that has
/// a perfectly good JS implementation sitting right next to it in
/// `bundle.js`.
///
/// This is the single source of truth for "is this name actually
/// callable via FFI" -- `generate_shim` and any caller that separately
/// needs to know which names require Fallback binding (e.g. thaw-cli's
/// `ModuleBundle::fallback_names`) both call this rather than
/// `classify_all` directly, so they can't disagree.
pub fn effective_classifications(
    functions: &[DtsFunction],
    native_lib_available: bool,
) -> Vec<(String, Classification)> {
    classify_all(functions)
        .into_iter()
        .map(|(name, classification)| match classification {
            Classification::FastPath(_) if !native_lib_available => (
                name.clone(),
                Classification::Fallback {
                    function: name,
                    reason: "no linked native library backs this Fast path signature".to_string(),
                },
            ),
            other => (name, other),
        })
        .collect()
}

/// `native_lib_available` says whether there's actually a linkable
/// native library backing this `.d.ts`'s FastPath signatures -- see
/// `effective_classifications`. thaw-cli passes `true` only for the
/// manual `--bridge` path, where the user is already responsible for
/// supplying a matching `--link`ed library themselves (that contract
/// predates this flag and is unchanged); for a registry (`--use`)
/// package it passes whether `native.a` actually exists.
/// A Fallback name thaw-cli also wants reachable through a package-
/// qualified Thaw-callable identifier, so a user can write `qs.parse(x)`
/// (rewritten by thaw-cli to call `alias`, since Thaw has no real
/// object/member-call support backing this -- it's pure source-level
/// syntax sugar) regardless of whether `parse` happens to collide with
/// another `--use`d package's own name. `qualified_key` is the JS-global
/// lookup key `callDynamic` uses at runtime, which must match the same
/// key `ModuleBundle::qualified_aliases` captured right after this
/// package's `loadScript` (`generate_module_init`) -- these always
/// travel together as a pair, generated by the same thaw-cli pass.
///
/// `suppress_bare` is set when `name` collides with another `--use`d
/// package's own declared name (thaw-cli detects this -- no single
/// `.d.ts`/package knows about any other package on its own): the bare
/// name is then *not* emitted at all, forcing qualified syntax, since
/// leaving it in would silently resolve to whichever colliding package
/// loaded last. When `false` (the common, non-colliding case), both the
/// bare name and the qualified alias are emitted, so a name stays
/// callable either way.
pub struct QualifiedFallback {
    pub name: String,
    pub alias: String,
    pub qualified_key: String,
    pub suppress_bare: bool,
}

/// `qualified` names (see `QualifiedFallback`) additionally get a
/// package-qualified alias function (and, when `suppress_bare` is set,
/// have their bare name dropped entirely -- see that field's doc
/// comment). A name not in `qualified` is emitted exactly as before.
pub fn generate_shim(
    functions: &[DtsFunction],
    native_lib_available: bool,
    qualified: &[QualifiedFallback],
) -> String {
    let mut out = String::new();
    for (name, classification) in effective_classifications(functions, native_lib_available) {
        match classification {
            Classification::FastPath(sig) => {
                // `classify_all` only returns `FastPath` for a name with
                // exactly one signature, so this lookup is unambiguous.
                let func = functions
                    .iter()
                    .find(|f| f.name == name)
                    .expect("FastPath classification implies a matching DtsFunction exists");
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
                let qualified_entry = qualified.iter().find(|q| q.name == function);
                if let Some(q) = qualified_entry {
                    out.push_str(&format!(
                        "// Fallback (QuickJS-NG), package-qualified alias: {reason}\n"
                    ));
                    out.push_str(&format!("function {}(argsArray: Json): Json {{\n", q.alias));
                    out.push_str(&format!(
                        "    return callDynamic(\"{}\", argsArray);\n",
                        q.qualified_key
                    ));
                    out.push_str("}\n");
                }
                if qualified_entry.is_some_and(|q| q.suppress_bare) {
                    continue;
                }
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

/// Escapes JS source for embedding as a double-quoted TS string literal
/// (backslash, `"`, and newlines/carriage-returns -- the characters that
/// would otherwise terminate or corrupt the literal). `generate_module_init`
/// is the only caller; a package's real `bundle.js` can contain any of
/// these.
fn escape_ts_string_literal(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for ch in source.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// One registry package's bundled JS to auto-load, for
/// `generate_module_init`.
pub struct ModuleBundle<'a> {
    pub package_name: &'a str,
    pub js_source: &'a str,
    /// This package's Fallback function names (from its `.d.ts`,
    /// `Classification::Fallback { function, .. }`) -- used by
    /// `wrap_as_commonjs_module` to bind a bare `module.exports = fn`
    /// default export (very common for small single-function utility
    /// packages, e.g. `left-pad`) to the name `callDynamic` will look it
    /// up by.
    pub fallback_names: &'a [String],
    /// `(bare_name, qualified_key)` pairs for names that collide with
    /// another `--use`d package (thaw-cli detects this across packages,
    /// which no single `.d.ts`/package knows about on its own). Right
    /// after this bundle's `loadScript`, `generate_module_init` also
    /// captures `globalThis[bare_name]` under `globalThis[qualified_key]`
    /// (e.g. `"qs::stringify"`) -- *before* a later colliding package's
    /// own `loadScript` overwrites the bare name -- so a package-
    /// qualified call (`qs.stringify(...)`, rewritten by thaw-cli to the
    /// matching `generate_shim` alias) reaches the right one regardless
    /// of `--use` order. See `generate_shim`'s `qualified_names` doc
    /// comment for the other half of this.
    pub qualified_aliases: &'a [(String, String)],
}

/// Wraps a real npm package's CommonJS source so it can run inside
/// QuickJS-NG's bare global scope: defines `module`/`exports`/`require`
/// as *globals* before the source runs (real CommonJS/UMD source
/// references them unconditionally, and QuickJS-NG's global scope has
/// none of them), runs the source completely unwrapped/at top level
/// (deliberately -- nesting it inside a function scope would break the
/// existing simple case of a hand-authored bundle using bare top-level
/// `function` declarations, which rely on top-level scope becoming
/// global properties directly, the same way `loadScript` already worked
/// before this), then copies whatever the source assigned to
/// `module.exports` onto the global scope too, so `callDynamic`'s
/// by-name lookup (`thaw_js_call`, which only ever looks up *global*
/// functions) can find it either way.
///
/// Validated by running real, unmodified npm packages through the
/// Fallback path: `left-pad` (`module.exports = leftPad`) and `slugify`
/// (a UMD wrapper that takes the CommonJS branch once `module`/`exports`
/// exist) both load and run correctly, and a plain hand-authored bundle
/// with no `module.exports` at all keeps working exactly as before.
/// `is-odd` -- which calls `require('is-number')` at load time -- fails
/// with a clear error instead of the pre-existing opaque
/// `ReferenceError: module is not defined`, correctly surfacing a real,
/// documented limitation (see below) rather than silently misbehaving.
///
/// `require` is a stub that throws immediately: resolving a real
/// inter-package dependency graph inside QuickJS-NG remains out of scope
/// (docs/design/bridge.md section 7's "未解決の論点"/"unresolved
/// questions"). This only unblocks packages with no runtime dependencies
/// of their own, which covers plenty of real small utility packages.
///
/// A `fallback_names` binding also checks `module.exports.default`, not
/// just `module.exports` itself, being a function: thaw-registry's ESM
/// rewrite (a real ESM package's `export default function foo(){}`)
/// always sets `module.exports.default`, matching real `import x from
/// 'y'` interop semantics, rather than replacing `module.exports`
/// outright the way a plain CommonJS `module.exports = foo` does.
fn wrap_as_commonjs_module(js_source: &str, fallback_names: &[String]) -> String {
    // `name` is always a valid JS identifier here: it's a function name
    // SWC already parsed out of a `.d.ts` `declare function` statement,
    // not arbitrary text, so splicing it directly as a property-access
    // identifier (not a bracketed string) is safe.
    let bind_default_exports: String = fallback_names
        .iter()
        .map(|name| {
            format!(
                "if (typeof module.exports === 'function') {{ globalThis.{name} = module.exports; }}\n\
                 else if (typeof module.exports === 'object' && module.exports !== null && typeof module.exports.default === 'function') {{ globalThis.{name} = module.exports.default; }}\n"
            )
        })
        .collect();
    format!(
        "globalThis.module = {{ exports: {{}} }};\n\
         globalThis.exports = globalThis.module.exports;\n\
         globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported in the Fallback path yet\"); }};\n\
         // Real packages commonly *guard* Node-only globals before using\n\
         // them (`Buffer && Buffer.isBuffer(x)`, `Buffer?.from(x)`) for\n\
         // exactly this situation -- a non-Node environment. But an\n\
         // undeclared bare identifier throws `ReferenceError` just from\n\
         // being *referenced*, guard or not (found via `@hapi/hoek`,\n\
         // which does this unconditionally at load time); explicitly\n\
         // assigning it `undefined` makes the identifier exist without\n\
         // pretending Buffer support exists, so those guards correctly\n\
         // take their \"not available\" branch instead of throwing.\n\
         if (typeof globalThis.Buffer === 'undefined') {{ globalThis.Buffer = undefined; }}\n\
         // Unlike `Buffer` above, real code sometimes reaches for `URL`\n\
         // *unconditionally* (e.g. `URL.prototype` as a lookup-table key,\n\
         // found via `@hapi/hoek`) rather than guarding it first --\n\
         // `undefined` doesn't survive a `.prototype` access, so this\n\
         // needs an actual (empty) constructor stand-in instead, which\n\
         // gets a `.prototype` object for free like any JS function.\n\
         if (typeof globalThis.URL === 'undefined') {{ globalThis.URL = function URL() {{}}; }}\n\
         // Node exposes `process` as an *ambient global*, not only as a\n\
         // requirable core module (thaw-registry's `builtin_module_source`\n\
         // covers the `require('process')` half) -- real packages\n\
         // (`node-gyp-build.js`, chasing a native addon's load path) read\n\
         // `process.config`/`process.env`/`process.versions`/\n\
         // `process.execPath` as a bare global with no guard at all, which\n\
         // would otherwise throw `ReferenceError: process is not defined`\n\
         // the same way an unguarded `Buffer`/`URL` reference would.\n\
         if (typeof globalThis.process === 'undefined') {{\n\
         \x20\x20globalThis.process = {{ argv: [], env: {{}}, platform: 'linux', version: '', execPath: '/usr/bin/node', config: {{ variables: {{}} }}, versions: {{ node: '', modules: '', uv: '' }}, nextTick: function(fn) {{ fn(); }} }};\n\
         }}\n\
         // Likewise `__dirname`/`__filename`: real per-module Node\n\
         // locals, but every package here already runs unwrapped at\n\
         // global scope (see this function's doc comment), so a shared\n\
         // global stand-in is consistent with the rest of this wrapper.\n\
         // The exact path is inert -- `fs` above always reports \"nothing\n\
         // here\", so no real lookup ever depends on this value being\n\
         // accurate, only present as a string.\n\
         if (typeof globalThis.__dirname === 'undefined') {{ globalThis.__dirname = '/thaw_modules/package'; }}\n\
         if (typeof globalThis.__filename === 'undefined') {{ globalThis.__filename = '/thaw_modules/package/index.js'; }}\n\
         {js_source}\n\
         if (typeof module.exports === 'object' && module.exports !== null) {{ for (var k in module.exports) {{ globalThis[k] = module.exports[k]; }} }}\n\
         {bind_default_exports}"
    )
}

/// Generates the `__thaw_module_init` function that loads each registry
/// package's bundled JS source via `loadScript`, once, before user code
/// runs (thaw-llvm's `MODULE_INIT_SYMBOL`/`call_module_init_if_present`
/// call this automatically from both entry points). This is what makes a
/// registry package's Fallback functions (see `generate_shim`) callable
/// without the *user* having to call `loadScript` themselves -- they still
/// need to `--use` the package (thaw-cli), but not hand-write the load.
/// Each bundle's JS is passed through `wrap_as_commonjs_module` first, so
/// a real npm package's actual CommonJS/UMD source works unmodified.
///
/// `bundles` is in the order packages should load (later packages can
/// rely on earlier ones already being loaded into the shared QuickJS-NG
/// global scope). Returns an empty string -- no function emitted at all
/// -- when `bundles` is empty, since there would be nothing for it to do.
pub fn generate_module_init(bundles: &[ModuleBundle]) -> String {
    if bundles.is_empty() {
        return String::new();
    }
    let mut out = String::from("function __thaw_module_init(): void {\n");
    for bundle in bundles {
        out.push_str(&format!("    // {}\n", bundle.package_name));
        let wrapped = wrap_as_commonjs_module(bundle.js_source, bundle.fallback_names);
        out.push_str(&format!(
            "    loadScript(\"{}\");\n",
            escape_ts_string_literal(&wrapped)
        ));
        // Captured immediately, before any later package's own
        // `loadScript` can overwrite the bare name -- see
        // `ModuleBundle::qualified_aliases`'s doc comment.
        for (bare_name, qualified_key) in bundle.qualified_aliases {
            // This alias-capture logic must run as *JS* (QuickJS-NG),
            // inside its own `loadScript`, not as bare Thaw source --
            // `__thaw_module_init` is a real, compiled Thaw function, and
            // Thaw's compiler has no `typeof`/bracket-indexing/etc. at
            // all. Double-escaped: `qualified_key` for the inner JS
            // string literal, then the whole JS snippet again for the
            // outer TS string literal `loadScript` takes, same as
            // `wrap_as_commonjs_module`'s own output already is.
            let capture_js = format!(
                "if (typeof globalThis.{bare_name} !== 'undefined') {{ globalThis[\"{}\"] = globalThis.{bare_name}; }}",
                escape_ts_string_literal(qualified_key)
            );
            out.push_str(&format!(
                "    loadScript(\"{}\");\n",
                escape_ts_string_literal(&capture_js)
            ));
        }
    }
    out.push_str("}\n");
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

    /// The exact shape found in a real npm package (`qs`): every function
    /// lives inside `declare namespace QueryString { ... }` rather than
    /// at the top level, with `export = QueryString;` outside it.
    #[test]
    fn extracts_functions_declared_inside_a_namespace() {
        let source = r#"
            export = QueryString;
            declare namespace QueryString {
                function stringify(obj: number): string;
                function parse(str: string): number;
            }
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 2);
        assert!(funcs.iter().any(|f| f.name == "stringify"));
        assert!(funcs.iter().any(|f| f.name == "parse"));
        // The bare name, not `QueryString.parse` -- that's what
        // `wrap_as_commonjs_module`'s object-export hoisting binds it to.
        assert!(!funcs.iter().any(|f| f.name.contains('.')));
    }

    #[test]
    fn extracts_functions_from_a_nested_namespace() {
        let source = r#"
            declare namespace Outer {
                namespace Inner {
                    function deep(x: number): number;
                }
                function shallow(x: number): number;
            }
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 2);
        assert!(funcs.iter().any(|f| f.name == "deep"));
        assert!(funcs.iter().any(|f| f.name == "shallow"));
    }

    /// The exact shape found in a real ESM npm package's `.d.ts`
    /// (`escape-string-regexp`): `export default function name(...): T;`
    /// is a different AST node (`ExportDefaultDecl`) than
    /// `export declare function name(...): T;` (`ExportDecl`); without
    /// handling it specifically, `parse_dts` found zero functions.
    #[test]
    fn extracts_a_named_export_default_function() {
        let source = "export default function escapeStringRegexp(string: string): string;";
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "escapeStringRegexp");
        assert!(matches!(classify(&funcs[0]), Classification::FastPath(_)));
    }

    /// An anonymous `export default function(...): T;` has no name to
    /// extract a callable `DtsFunction` under -- must be silently
    /// skipped, not panic.
    #[test]
    fn anonymous_export_default_function_is_skipped_not_panicked_on() {
        let source = "export default function(string: string): string;";
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 0);
    }

    #[test]
    fn namespaced_functions_classify_normally() {
        let source = r#"
            declare namespace Ns {
                function add(a: number, b: number): number;
                function identity<T>(x: T): T;
            }
        "#;
        let funcs = parse_dts(source).unwrap();
        let add = funcs.iter().find(|f| f.name == "add").unwrap();
        let identity = funcs.iter().find(|f| f.name == "identity").unwrap();
        assert!(matches!(classify(add), Classification::FastPath(_)));
        assert!(matches!(classify(identity), Classification::Fallback { .. }));
    }

    /// The real `qs` round-trip: a namespaced Fallback function's
    /// generated wrapper must actually parse and lower, same bar as
    /// every other `generate_shim` round-trip test.
    #[test]
    fn namespaced_fallback_function_shim_round_trips_through_real_lowering() {
        let source = r#"
            export = QueryString;
            declare namespace QueryString {
                function parse(str: string, options?: object): object;
            }
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 1);
        let shim = generate_shim(&funcs, false, &[]);
        assert!(shim.contains("function parse(argsArray: Json): Json {"));

        let program_source = format!(
            "{shim}\nfunction main(): void {{\n    const r = parse(JSON.parse(\"[\\\"a=1\\\"]\"));\n    console.log(String(r));\n}}\n"
        );
        let module = thaw_parser::parse_typescript(&program_source)
            .unwrap_or_else(|e| panic!("generated shim did not parse: {e}\n---\n{program_source}"));
        thaw_hir::lower_module(&module)
            .unwrap_or_else(|e| panic!("generated shim did not lower: {e}\n---\n{program_source}"));
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
            generate_shim(&funcs, true, &[]),
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
            generate_shim(&funcs, true, &[]),
            "declare function dist(p: { x: number; y: number }): number;\n"
        );
    }

    /// The actual bug: a real npm package fetched via `thaw registry
    /// add` (e.g. date-fns's `daysToWeeks(days: number): number`) is
    /// pure JS with no native library at all, but classifies FastPath on
    /// type shape alone. Emitting the ambient `declare function` anyway
    /// produced a real, reproduced `undefined reference` linker error --
    /// `native_lib_available: false` must downgrade it to a Fallback
    /// wrapper instead, since a working JS implementation is right there.
    #[test]
    fn downgrades_fast_path_to_fallback_when_no_native_lib_is_available() {
        let funcs = parse_dts("export declare function daysToWeeks(days: number): number;").unwrap();
        let shim = generate_shim(&funcs, false, &[]);
        assert!(
            !shim.contains("declare function"),
            "must not emit an ambient FFI declaration with nothing to link against, got:\n{shim}"
        );
        assert!(shim.contains("function daysToWeeks(argsArray: Json): Json {"));
        assert!(shim.contains(r#"return callDynamic("daysToWeeks", argsArray);"#));
    }

    #[test]
    fn native_lib_available_true_keeps_fast_path_as_before() {
        let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
        assert_eq!(
            generate_shim(&funcs, true, &[]),
            "declare function add(a: number, b: number): number;\n"
        );
    }

    #[test]
    fn effective_classifications_downgrades_fast_path_when_native_lib_unavailable() {
        let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
        let result = effective_classifications(&funcs, false);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].1, Classification::Fallback { .. }));
    }

    #[test]
    fn effective_classifications_leaves_fallback_alone_regardless_of_native_lib() {
        let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
        let with_native = effective_classifications(&funcs, true);
        let without_native = effective_classifications(&funcs, false);
        assert!(matches!(with_native[0].1, Classification::Fallback { .. }));
        assert!(matches!(without_native[0].1, Classification::Fallback { .. }));
    }

    #[test]
    fn generates_call_dynamic_wrapper_for_fallback_function() {
        let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
        let shim = generate_shim(&funcs, true, &[]);
        assert!(shim.contains("// Fallback (QuickJS-NG):"));
        assert!(shim.contains("function identity(argsArray: Json): Json {"));
        assert!(shim.contains(r#"return callDynamic("identity", argsArray);"#));
    }

    #[test]
    fn classify_all_collapses_a_single_signature_exactly_like_classify() {
        let funcs = parse_dts("export declare function add(a: number, b: number): number;").unwrap();
        let grouped = classify_all(&funcs);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0], ("add".to_string(), classify(&funcs[0])));
    }

    /// The exact shape found in a real npm package (`@types/ms`): two
    /// `declare function ms(...)` overloads, one classifying FastPath on
    /// its own and one Fallback. Thaw can't represent two native
    /// signatures under one FFI symbol, so the whole name must fall back.
    #[test]
    fn classify_all_falls_back_an_overload_set_even_if_one_member_is_fast_path() {
        let dts = r#"
            declare function ms(value: number, options?: { long: boolean }): string;
            declare function ms(value: string): number;
        "#;
        let funcs = parse_dts(dts).unwrap();
        assert_eq!(funcs.len(), 2, "both overloads should be extracted");

        let grouped = classify_all(&funcs);
        assert_eq!(grouped.len(), 1, "one name -> one classification, not two");
        let (name, classification) = &grouped[0];
        assert_eq!(name, "ms");
        assert!(
            matches!(classification, Classification::Fallback { .. }),
            "an overloaded name must fall back even if one overload alone would be FastPath"
        );
    }

    /// Same rule even when *every* overload individually classifies
    /// FastPath: Thaw still can't pick one signature over the other, so
    /// this must not silently choose either.
    #[test]
    fn classify_all_falls_back_when_every_overload_is_individually_fast_path() {
        let dts = r#"
            declare function f(a: number): number;
            declare function f(a: string): string;
        "#;
        let funcs = parse_dts(dts).unwrap();
        let grouped = classify_all(&funcs);
        assert_eq!(grouped.len(), 1);
        assert!(matches!(grouped[0].1, Classification::Fallback { .. }));
    }

    /// Regression test for the actual bug: before `classify_all`,
    /// `generate_shim` emitted one entry per raw `DtsFunction`, so an
    /// overloaded name produced two colliding top-level declarations
    /// (`declare function ms(...)` and `function ms(argsArray...) {...}`)
    /// that thaw-hir/codegen resolved silently and order-dependently.
    #[test]
    fn generate_shim_emits_exactly_one_declaration_for_an_overloaded_name() {
        let dts = r#"
            declare function ms(value: number, options?: { long: boolean }): string;
            declare function ms(value: string): number;
        "#;
        let funcs = parse_dts(dts).unwrap();
        let shim = generate_shim(&funcs, true, &[]);

        assert_eq!(
            shim.matches("function ms").count(),
            1,
            "exactly one top-level declaration named `ms`, got:\n{shim}"
        );
        assert!(shim.contains("// Fallback (QuickJS-NG):"));
        assert!(shim.contains(r#"return callDynamic("ms", argsArray);"#));
    }

    /// A name in `qualified` with `suppress_bare: true` is emitted only
    /// under its alias, calling `callDynamic` with the qualified key --
    /// not the bare name -- and the bare name isn't emitted as a
    /// declaration at all (thaw-cli sets this once it's decided the bare
    /// identifier must not be directly callable, to force qualified
    /// syntax after an actual cross-package collision).
    #[test]
    fn generates_qualified_alias_for_a_cross_package_colliding_name() {
        let funcs = parse_dts("export declare function stringify<T>(x: T): T;").unwrap();
        let qualified = [QualifiedFallback {
            name: "stringify".to_string(),
            alias: "qs_stringify".to_string(),
            qualified_key: "qs::stringify".to_string(),
            suppress_bare: true,
        }];
        let shim = generate_shim(&funcs, true, &qualified);

        assert!(shim.contains("function qs_stringify(argsArray: Json): Json {"));
        assert!(shim.contains(r#"return callDynamic("qs::stringify", argsArray);"#));
        assert!(!shim.contains("function stringify(argsArray: Json): Json {"));
    }

    /// `suppress_bare: false` (the common, non-colliding case) emits
    /// *both* the bare name and the qualified alias -- a name stays
    /// callable either way (`parse(x)` or `qs.parse(x)`).
    #[test]
    fn generates_both_bare_and_qualified_alias_when_not_suppressed() {
        let funcs = parse_dts("export declare function identity<T>(x: T): T;").unwrap();
        let qualified = [QualifiedFallback {
            name: "identity".to_string(),
            alias: "qs_identity".to_string(),
            qualified_key: "qs::identity".to_string(),
            suppress_bare: false,
        }];
        let shim = generate_shim(&funcs, true, &qualified);

        assert!(shim.contains("function qs_identity(argsArray: Json): Json {"));
        assert!(shim.contains(r#"return callDynamic("qs::identity", argsArray);"#));
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
        let shim = generate_shim(&funcs, true, &[]);

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

    #[test]
    fn generate_module_init_is_empty_for_no_bundles() {
        assert_eq!(generate_module_init(&[]), "");
    }

    #[test]
    fn generates_load_script_call_per_bundle() {
        let bundles = [
            ModuleBundle {
                package_name: "left-pad",
                js_source: "function pad(s) { return s; }",
                fallback_names: &[],
                qualified_aliases: &[],
            },
            ModuleBundle {
                package_name: "is-odd",
                js_source: "function isOdd(n) { return n % 2 === 1; }",
                fallback_names: &[],
                qualified_aliases: &[],
            },
        ];
        let init = generate_module_init(&bundles);
        assert!(init.starts_with("function __thaw_module_init(): void {\n"));
        assert!(init.contains("function pad(s) { return s; }"));
        assert!(init.contains("function isOdd(n) { return n % 2 === 1; }"));
        // Loaded in the given order.
        assert!(init.find("left-pad").unwrap() < init.find("is-odd").unwrap());
    }

    #[test]
    fn escapes_quotes_and_newlines_in_bundled_source() {
        let bundles = [ModuleBundle {
            package_name: "pkg",
            js_source: "function f() {\n  return \"a\\b\";\n}",
            fallback_names: &[],
            qualified_aliases: &[],
        }];
        let init = generate_module_init(&bundles);
        // The wrapper adds its own quotes/backslashes/newlines too; this
        // just confirms the *bundle's own* problematic characters survived
        // escaping correctly once embedded inside the wrapped script.
        assert!(init.contains(r#"return \"a\\b\";\n"#));
    }

    #[test]
    fn wraps_real_commonjs_source_and_binds_default_export() {
        // The exact shape of left-pad's actual published `index.js`:
        // `module.exports = leftPad;`, no named exports object.
        let js_source = "module.exports = function leftPad(str) { return str; };";
        let wrapped = wrap_as_commonjs_module(js_source, &["leftPad".to_string()]);
        assert!(wrapped.contains("globalThis.module = { exports: {} };"));
        assert!(wrapped.contains("globalThis.require ="));
        assert!(wrapped.contains(js_source));
        assert!(wrapped.contains("globalThis.leftPad = module.exports;"));
    }

    /// The exact shape thaw-registry's ESM rewrite produces for a real
    /// ESM package (`escape-string-regexp`'s `export default function
    /// escapeStringRegexp(){}`): `module.exports.default = <fn>`, not
    /// `module.exports = <fn>` directly. Runs through real QuickJS-NG to
    /// confirm the Fallback name actually ends up callable, not just
    /// that the generated text looks plausible.
    #[test]
    fn binds_esm_default_export_under_the_fallback_name() {
        use std::ffi::{CStr, CString};

        let js_source = "module.exports.__esModule = true;\nmodule.exports.default = function escapeIt(s) { return '[' + s + ']'; };";
        let wrapped = wrap_as_commonjs_module(js_source, &["escapeIt".to_string()]);

        let source = CString::new(wrapped).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "failed to load");

        let func = CString::new("escapeIt").unwrap();
        let args = CString::new("[\"hi\"]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
        assert_eq!(result, "\"[hi]\"");
    }

    #[test]
    fn bare_global_function_bundle_is_unaffected_by_commonjs_wrapping() {
        // A hand-authored bundle with no `module.exports` at all (this
        // session's registry examples before real npm packages were
        // tested) must keep defining a plain global function, not get
        // hidden inside a nested scope.
        let wrapped = wrap_as_commonjs_module(
            "function greet(name) { return 'hi, ' + name; }",
            &["greet".to_string()],
        );
        assert!(wrapped.contains("function greet(name) { return 'hi, ' + name; }"));
        // Not wrapped in an extra IIFE/function around the source itself.
        assert!(!wrapped.contains("(function(module, exports, require)"));
    }

    /// The exact pattern found in a real npm package (`@hapi/hoek`):
    /// code that *guards* a Node-only global before using it (`Buffer &&
    /// Buffer.isBuffer(x)`) still throws `ReferenceError: Buffer is not
    /// defined` if `Buffer` was never declared anywhere -- referencing an
    /// undeclared bare identifier throws regardless of the guard's
    /// intent. Must load successfully and take the guard's "not
    /// available" branch instead.
    #[test]
    fn guarded_buffer_reference_does_not_throw() {
        use std::ffi::{CStr, CString};

        let wrapped = wrap_as_commonjs_module(
            "module.exports = function checkBuffer(x) { return (Buffer && Buffer.isBuffer(x)) || false; };",
            &["checkBuffer".to_string()],
        );

        let source = CString::new(wrapped).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "failed to load");

        let func = CString::new("checkBuffer").unwrap();
        let args = CString::new("[1]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
        assert_eq!(result, "false", "the guard should take its no-Buffer branch, not throw");
    }

    /// The exact pattern found in the same real npm package (`@hapi/hoek`):
    /// `URL.prototype` accessed *unconditionally* (as a lookup-table key,
    /// not behind a truthiness guard) -- `undefined` doesn't survive a
    /// `.prototype` property access the way it survives `Buffer && ...`,
    /// so `URL` needs an actual (empty) constructor stand-in instead.
    #[test]
    fn unguarded_url_prototype_access_does_not_throw() {
        use std::ffi::CString;

        let wrapped = wrap_as_commonjs_module(
            "module.exports = function getIt() { return typeof URL.prototype; };",
            &["getIt".to_string()],
        );

        let source = CString::new(wrapped).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "failed to load");
    }

    /// The exact pattern chasing a real native addon's load path
    /// (`bcrypt`, via its `node-gyp-build` dependency's real, unmodified
    /// `node-gyp-build.js`): several bare `process.*` reads with no
    /// `require('process')` and no guard at all -- `process` needs to
    /// exist as an ambient *global*, not just as thaw-registry's
    /// requirable `process` module, or this throws `ReferenceError:
    /// process is not defined` the same way an unguarded `Buffer`/`URL`
    /// reference would without their global stand-ins.
    #[test]
    fn unguarded_process_global_reference_does_not_throw() {
        use std::ffi::{CStr, CString};

        let wrapped = wrap_as_commonjs_module(
            "module.exports = function readIt() {\n\
             \x20\x20var vars = (process.config && process.config.variables) || {};\n\
             \x20\x20var abi = process.versions.modules;\n\
             \x20\x20var uv = (process.versions.uv || '').split('.')[0];\n\
             \x20\x20return typeof process.env + ',' + typeof process.execPath + ',' + typeof __dirname + ',' + typeof __filename;\n\
             };",
            &["readIt".to_string()],
        );

        let source = CString::new(wrapped).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1, "failed to load");

        let func = CString::new("readIt").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned();
        assert_eq!(result, "\"object,string,string,string\"");
    }

    /// Same bar as `generated_shim_round_trips_through_real_lowering`: the
    /// generated `__thaw_module_init` must actually be valid, lowerable
    /// Thaw source, not just plausible text.
    #[test]
    fn generated_module_init_round_trips_through_real_lowering() {
        let fallback_names = vec!["greet".to_string()];
        let init = generate_module_init(&[ModuleBundle {
            package_name: "greeter",
            js_source: "function greet(){return 'hi';}",
            fallback_names: &fallback_names,
            qualified_aliases: &[],
        }]);
        let program_source = format!(
            "{init}\nfunction main(): void {{\n    console.log(String(callDynamic(\"greet\", JSON.parse(\"[]\"))));\n}}\n"
        );

        let module = thaw_parser::parse_typescript(&program_source)
            .unwrap_or_else(|e| panic!("generated module_init did not parse: {e}\n---\n{program_source}"));
        let program = thaw_hir::lower_module(&module)
            .unwrap_or_else(|e| panic!("generated module_init did not lower: {e}\n---\n{program_source}"));

        assert_eq!(program.functions.len(), 2);
        assert!(program.functions.iter().any(|f| f.name == "__thaw_module_init"));
        assert!(program.functions.iter().any(|f| f.name == "main"));
    }

    /// A bundle's `qualified_aliases` capture the bare name under the
    /// qualified key *immediately* after that bundle's own `loadScript`
    /// -- before a later, colliding package's `loadScript` gets a chance
    /// to overwrite the bare name.
    #[test]
    fn generate_module_init_captures_qualified_aliases_right_after_load() {
        let qs_fallback = vec!["stringify".to_string()];
        let qs_aliases = vec![("stringify".to_string(), "qs::stringify".to_string())];
        let hoek_fallback = vec!["stringify".to_string()];
        let hoek_aliases = vec![("stringify".to_string(), "hoek::stringify".to_string())];
        let bundles = [
            ModuleBundle {
                package_name: "qs",
                js_source: "module.exports = { stringify: function(x) { return 'qs:' + x; } };",
                fallback_names: &qs_fallback,
                qualified_aliases: &qs_aliases,
            },
            ModuleBundle {
                package_name: "@hapi/hoek",
                js_source: "module.exports = { stringify: function(x) { return 'hoek:' + x; } };",
                fallback_names: &hoek_fallback,
                qualified_aliases: &hoek_aliases,
            },
        ];
        let init = generate_module_init(&bundles);

        // Each capture line must appear *between* its own package's
        // loadScript and the next package's, not after both have loaded.
        // The capture JS is embedded (and so escaped) inside its own
        // `loadScript("...")` TS string literal -- a literal backslash
        // precedes each quote in the *generated Thaw source text*, not
        // just a bare `"`.
        let qs_load = init.find("// qs").unwrap();
        let qs_capture = init.find(r#"globalThis[\"qs::stringify\"]"#).unwrap();
        let hoek_load = init.find("// @hapi/hoek").unwrap();
        let hoek_capture = init.find(r#"globalThis[\"hoek::stringify\"]"#).unwrap();
        assert!(qs_load < qs_capture, "qs's capture must come after qs's own load");
        assert!(qs_capture < hoek_load, "qs's capture must come before hoek's load");
        assert!(hoek_load < hoek_capture);
    }

    /// Found by running the classifier against a real npm package's
    /// `.d.ts` corpus (date-fns): before `describe_ts_type`, this reason
    /// was an unreadable `{:?}` dump of the full nested AST (spans,
    /// `Box`es, and all) instead of anything resembling what a developer
    /// wrote.
    #[test]
    fn fallback_reason_renders_union_types_readably() {
        let funcs = parse_dts("export declare function f(x: string | number): void;").unwrap();
        let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
            panic!("expected Fallback");
        };
        assert_eq!(reason, "parameter `x`: unsupported type `string | number`");
    }

    /// The exact shape found in date-fns v4's real `.d.ts` files (e.g.
    /// `addDays`'s `date: DateArg<DateType> & {}` parameter).
    #[test]
    fn fallback_reason_renders_intersection_and_generic_types_readably() {
        let funcs =
            parse_dts("export declare function f(x: DateArg<Date> & {}): void;").unwrap();
        let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
            panic!("expected Fallback");
        };
        assert_eq!(reason, "parameter `x`: unsupported type `DateArg<Date> & {}`");
    }

    #[test]
    fn fallback_reason_renders_callback_types_readably() {
        let funcs =
            parse_dts("export declare function f(cb: (err: string) => void): void;").unwrap();
        let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
            panic!("expected Fallback");
        };
        assert_eq!(reason, "parameter `cb`: unsupported type `a function type`");
    }

    #[test]
    fn fallback_reason_renders_unsupported_keywords_by_name() {
        let funcs = parse_dts("export declare function f(x: any): void;").unwrap();
        let Classification::Fallback { reason, .. } = classify(&funcs[0]) else {
            panic!("expected Fallback");
        };
        assert_eq!(reason, "parameter `x`: `any` is not supported");
    }

    /// The actual regression this was validated against: a real date-fns
    /// function (`milliseconds({ years, months, ... }: Duration)`) uses a
    /// destructured parameter, which `parse_dts` used to reject by
    /// returning `Err` for the *whole file* -- silently discarding every
    /// other function defined alongside it. It must now still show up
    /// (correctly, as Fallback), and every sibling function in the same
    /// file must survive.
    #[test]
    fn destructured_parameter_falls_back_without_dropping_sibling_functions() {
        let source = r#"
            export declare function add(a: number, b: number): number;
            export declare function milliseconds({ hours, minutes }: Duration): number;
            export declare function subtract(a: number, b: number): number;
        "#;
        let funcs = parse_dts(source).unwrap();
        assert_eq!(funcs.len(), 3, "one bad parameter pattern must not delete sibling functions");

        assert!(matches!(
            classify(funcs.iter().find(|f| f.name == "add").unwrap()),
            Classification::FastPath(_)
        ));
        assert!(matches!(
            classify(funcs.iter().find(|f| f.name == "subtract").unwrap()),
            Classification::FastPath(_)
        ));

        let Classification::Fallback { reason, .. } =
            classify(funcs.iter().find(|f| f.name == "milliseconds").unwrap())
        else {
            panic!("expected Fallback");
        };
        assert!(reason.contains("unsupported parameter pattern"));
    }
}
