//! `.d.ts` -> Fast-path/Fallback classification. See
//! `docs/design/bridge.md` for the full design; this crate implements
//! sections 3 (parsing) and 4 (type classification) of it.
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

use swc_ecma_ast::{
    Decl, Expr, FnDecl, ModuleDecl, ModuleItem, Pat, TsEntityName, TsKeywordTypeKind, TsType,
    TsTypeElement,
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
    /// `interface` types, ...). Carries a human-readable reason.
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
/// is equally ambient here). Anything else at the top level (classes,
/// `interface`, `const`, re-exports, ...) is silently skipped: this is a
/// function-signature extractor, not a full `.d.ts` model.
pub fn parse_dts(source: &str) -> Result<Vec<DtsFunction>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    module
        .body
        .iter()
        .filter_map(extract_fn_decl)
        .map(lower_dts_function)
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

fn lower_dts_function(fn_decl: &FnDecl) -> Result<DtsFunction, String> {
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
                Some(ann) => classify_ts_type(&ann.type_ann),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            Ok((param_name, ty))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let ret = match &func.return_type {
        Some(ann) => classify_ts_type(&ann.type_ann),
        None => DtsType::Native(HirType::Void),
    };

    Ok(DtsFunction { name, params, ret })
}

/// Mirrors `thaw_hir::lower::lower_ts_type`'s mapping rules, but never
/// fails: anything it can't map becomes `DtsType::Unsupported` with a
/// reason, for `classify` to report per-parameter/return instead of
/// aborting the whole `.d.ts` file over one unsupported signature.
fn classify_ts_type(ty: &TsType) -> DtsType {
    match ty {
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => DtsType::Native(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => DtsType::Native(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => DtsType::Native(HirType::Bool),
            TsKeywordTypeKind::TsVoidKeyword => DtsType::Native(HirType::Void),
            other => DtsType::Unsupported(format!("unsupported keyword type {other:?}")),
        },

        TsType::TsArrayType(arr) => match classify_ts_type(&arr.elem_type) {
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
                    Some(ann) => classify_ts_type(&ann.type_ann),
                    None => DtsType::Unsupported(format!("field `{field_name}` has no type annotation")),
                };
                match field_ty {
                    DtsType::Native(HirType::F64) => fields.push((field_name, HirType::F64)),
                    DtsType::Native(other) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}` has type {other:?} (only number fields supported yet)"
                        ))
                    }
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
            // Note: no `Array<T>`/`Promise<T>` recognition here (unlike
            // thaw-hir's `lower_ts_type`) -- a `.d.ts` signature using
            // either still needs a real decision about arena lifetime
            // (arrays) or the async ABI (promises) across a *foreign* FFI
            // boundary that section 5 of the design doc explicitly defers.
            DtsType::Unsupported(format!(
                "type reference `{ref_name}` is not classified yet (generics/interfaces/Array<T>/Promise<T>)"
            ))
        }

        other => DtsType::Unsupported(format!("unsupported type {other:?}")),
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
}
