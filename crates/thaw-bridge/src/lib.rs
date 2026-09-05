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

use std::collections::{HashMap, HashSet};

use swc_ecma_ast::{
    Accessibility, Class, ClassMember, Decl, DefaultDecl, Expr, Function, MethodKind, Module,
    ModuleDecl, ModuleItem, ParamOrTsParamProp, Pat, PropName, TruePlusMinus, TsCallSignatureDecl,
    TsEntityName, TsFnOrConstructorType, TsFnParam, TsFnType, TsInterfaceDecl, TsKeywordTypeKind,
    TsLit, TsMethodSignature, TsNamespaceBody, TsParamPropParam, TsType, TsTypeElement,
    TsTypeOperatorOp, TsUnionOrIntersectionType,
};
use thaw_hir::{
    FfiAggregateAbi, FfiCallingConvention, FfiErrorAbi, FfiOwnership, FfiSignature, FfiStringAbi,
    HirOptionalMask, HirType,
};

/// One function signature extracted from a `.d.ts` file, before
/// classification.
#[derive(Debug, Clone, PartialEq)]
pub struct DtsFunction {
    pub name: String,
    pub generic: Option<DtsGenericFunction>,
    pub params: Vec<(String, DtsType)>,
    pub required_params: usize,
    pub rest_param: Option<(String, DtsType)>,
    pub ret: DtsType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DtsGenericFunction {
    pub type_params: Vec<(String, Option<String>)>,
    pub param_types: Vec<String>,
    /// The declared return type, rendered the same crude way
    /// `param_types` are (`describe_ts_type`) -- used to recognize the
    /// specific, safe-to-preserve shape of a type parameter that appears
    /// *only* in the return position (e.g. `nanoid<Type extends string>
    /// (size?: number): Type`), where this is exactly one of
    /// `type_params`' own names.
    pub return_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DtsClass {
    pub name: String,
    pub extends: Option<String>,
    pub constructible: bool,
    pub constructors: Vec<DtsConstructor>,
    pub methods: Vec<DtsMethod>,
    pub properties: Vec<DtsProperty>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DtsConstructor {
    pub params: Vec<(String, DtsType)>,
    pub required_params: usize,
    pub overloaded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DtsMethod {
    pub name: String,
    pub params: Vec<(String, DtsType)>,
    /// Number of parameters that must be present at a call site. Any
    /// remaining trailing parameters were marked optional in the `.d.ts`.
    pub required_params: usize,
    /// A trailing rest parameter, stored as its element type rather than
    /// the array type written in TypeScript.
    pub rest_param: Option<(String, DtsType)>,
    pub ret: DtsType,
    pub is_static: bool,
    pub kind: DtsMethodKind,
    pub overloaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtsMethodKind {
    Method,
    Getter,
    Setter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DtsProperty {
    pub name: String,
    pub ty: DtsType,
    pub is_static: bool,
    pub readonly: bool,
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
    FastPath(Box<FfiSignature>),
    /// At least one parameter or the return type didn't map -- needs the
    /// QuickJS-NG fallback path (docs/design/bridge.md section 7, not
    /// implemented yet).
    Fallback { function: String, reason: String },
}

include!("bridge/dts.rs");

include!("bridge/classification.rs");

include!("bridge/generation.rs");

#[cfg(test)]
mod tests;
