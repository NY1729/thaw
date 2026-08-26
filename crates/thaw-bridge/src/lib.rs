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
    Class, ClassMember, Decl, DefaultDecl, Expr, Function, MethodKind, Module, ModuleDecl,
    ModuleItem, ParamOrTsParamProp, Pat, PropName, TsEntityName, TsFnOrConstructorType, TsFnParam,
    TsInterfaceDecl, TsKeywordTypeKind, TsLit, TsNamespaceBody, TsParamPropParam, TsType,
    TsTypeElement, TsTypeOperatorOp, TsUnionOrIntersectionType,
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
    pub params: Vec<(String, DtsType)>,
    pub rest_param: Option<(String, DtsType)>,
    pub ret: DtsType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DtsClass {
    pub name: String,
    pub extends: Option<String>,
    pub constructors: Vec<DtsConstructor>,
    pub methods: Vec<DtsMethod>,
    pub properties: Vec<DtsProperty>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DtsConstructor {
    pub params: Vec<(String, DtsType)>,
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

pub fn parse_dts_classes(source: &str) -> Result<Vec<DtsClass>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let mut classes = module
        .body
        .iter()
        .filter_map(extract_class_decl)
        .map(|(name, class)| lower_dts_class(name, class, &interfaces, &generic_interfaces))
        .collect::<Vec<_>>();
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

fn extract_class_decl(item: &ModuleItem) -> Option<(&str, &Class)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Class(class))) => {
            Some((class.ident.sym.as_str(), &class.class))
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::Class(class) => Some((class.ident.sym.as_str(), &class.class)),
            _ => None,
        },
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Class(class) => class
                .ident
                .as_ref()
                .map(|ident| (ident.sym.as_str(), class.class.as_ref())),
            _ => None,
        },
        _ => None,
    }
}

fn property_name(key: &PropName) -> Option<String> {
    match key {
        PropName::Ident(name) => Some(name.sym.to_string()),
        PropName::Str(name) => Some(name.value.to_string_lossy().into_owned()),
        _ => None,
    }
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
                    _ => None,
                },
                ParamOrTsParamProp::TsParamProp(property) => match &property.param {
                    TsParamPropParam::Ident(binding) => Some(binding),
                    TsParamPropParam::Assign(_) => None,
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
    for member in &class.body {
        match member {
            ClassMember::Constructor(constructor) => constructors.push(DtsConstructor {
                params: lower_class_params(&constructor.params, interfaces, generic_interfaces),
                overloaded: false,
            }),
            ClassMember::Method(method) => {
                let Some(method_name) = property_name(&method.key) else {
                    continue;
                };
                let function = lower_dts_function(
                    &method_name,
                    &method.function,
                    interfaces,
                    generic_interfaces,
                );
                let params = function.params;
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
            ClassMember::ClassProp(property) => {
                let Some(property_name) = property_name(&property.key) else {
                    continue;
                };
                let ty = property
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                    })
                    .unwrap_or_else(|| DtsType::Unsupported("missing type annotation".into()));
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
                Some(ann) => {
                    resolve_type_with_interfaces(&ann.type_ann, raw, resolved, in_progress)
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
                    classify_variadic_ts_type(&array.elem_type, interfaces, generic_interfaces)
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

    DtsFunction {
        name,
        params,
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

        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Number(_) => DtsType::Native(HirType::F64),
            TsLit::Str(_) => DtsType::Native(HirType::Str),
            TsLit::Bool(_) => DtsType::Native(HirType::Bool),
            _ => DtsType::Unsupported("unsupported literal type".into()),
        },

        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let mut native = None;
            for element in &union.types {
                match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(ty) if native.as_ref().is_none_or(|current| current == &ty) => {
                        native = Some(ty);
                    }
                    DtsType::Native(_) => {
                        return DtsType::Unsupported(format!(
                            "unsupported type `{}`",
                            describe_ts_type(ty)
                        ))
                    }
                    unsupported => return unsupported,
                }
            }
            native
                .map(DtsType::Native)
                .unwrap_or_else(|| DtsType::Unsupported("empty union type is not supported".into()))
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
                DtsType::Native(HirType::F64) => {
                    DtsType::Native(HirType::Array(Box::new(HirType::F64)))
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "array element type {other:?} is not supported yet (only number[])"
                )),
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("array element type: {reason}"))
                }
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
                            DtsType::Unsupported(reason) => {
                                return DtsType::Unsupported(format!(
                                    "callback rest element type: {reason}"
                                ));
                            }
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
            match classify_ts_type(&function.type_ann.type_ann, interfaces, generic_interfaces) {
                DtsType::Native(ret) => {
                    DtsType::Native(if rest.is_some() || optional.iter().any(|value| *value) {
                        let optional = HirOptionalMask::from_bools(&optional);
                        HirType::CallableFunction(
                            params,
                            optional,
                            rest.map(Box::new),
                            Box::new(ret),
                        )
                    } else {
                        HirType::Function(params, Box::new(ret))
                    })
                }
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("callback return type: {reason}"))
                }
            }
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(_)) => {
            DtsType::Unsupported("constructor callback types are not supported".into())
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
                    _ => {
                        return DtsType::Unsupported(
                            "unsupported object type literal key".to_string(),
                        )
                    }
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
                    DtsType::Native(ty) => fields.push((field_name, ty)),
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}`: {reason}"
                        ))
                    }
                }
            }
            DtsType::Native(HirType::Object(fields))
        }

        TsType::TsTypeRef(ty_ref) => {
            let ref_name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(_) => {
                    return DtsType::Unsupported(
                        "qualified type names are not supported yet".to_string(),
                    )
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
            if ref_name == "Json" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::Json);
            }
            if ref_name == "JsValue" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::JsValue);
            }
            if ref_name == "Array" {
                let Some(element) = ty_ref
                    .type_params
                    .as_ref()
                    .and_then(|params| params.params.first())
                else {
                    return DtsType::Unsupported("Array<T> needs one type argument".to_string());
                };
                return match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(HirType::F64) => {
                        DtsType::Native(HirType::Array(Box::new(HirType::F64)))
                    }
                    _ => DtsType::Unsupported(
                        "only number[] has a native dynamic layout".to_string(),
                    ),
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
                DtsType::Native(HirType::F64) => {
                    DtsType::Native(HirType::Array(Box::new(HirType::F64)))
                }
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
                    _ => {
                        return DtsType::Unsupported(
                            "unsupported object type literal key".to_string(),
                        )
                    }
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
                    DtsType::Native(ty) => fields.push((field_name, ty)),
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}`: {reason}"
                        ))
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

    let variadic = match &func.rest_param {
        None => None,
        Some((_, DtsType::Native(ty))) if supports_variadic_element(ty) => Some(ty.clone()),
        Some((name, DtsType::Native(other))) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: format!(
                "rest parameter `{name}` has unsupported native variadic element layout {other:?}"
            ),
            }
        }
        Some((name, DtsType::Unsupported(reason))) => {
            return Classification::Fallback {
                function: func.name.clone(),
                reason: format!("rest parameter `{name}`: {reason}"),
            }
        }
    };

    Classification::FastPath(Box::new(FfiSignature {
        symbol: func.name.clone(),
        params,
        variadic,
        variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        ret,
        error_abi: FfiErrorAbi::Direct,
        return_ownership: FfiOwnership::Borrowed,
        error_ownership: FfiOwnership::Borrowed,
        param_string_abis: vec![FfiStringAbi::NullTerminated; func.params.len()],
        return_string_abi: FfiStringAbi::NullTerminated,
        calling_convention: FfiCallingConvention::C,
        aggregate_return_abi: FfiAggregateAbi::Internal,
        aggregate_return_layout: None,
    }))
}

fn supports_variadic_element(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue => true,
        HirType::Array(element) => matches!(
            element.as_ref(),
            HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue
        ),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            supports_variadic_element(payload)
        }
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supports_variadic_element(field)),
        _ => false,
    }
}

fn classify_variadic_ts_type(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsType {
    if let TsType::TsParenthesizedType(parenthesized) = ty {
        return classify_variadic_ts_type(&parenthesized.type_ann, interfaces, generic_interfaces);
    }
    if let TsType::TsArrayType(array) = ty {
        return match classify_ts_type(&array.elem_type, interfaces, generic_interfaces) {
            DtsType::Native(
                element @ (HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue),
            ) => DtsType::Native(HirType::Array(Box::new(element))),
            DtsType::Native(other) => DtsType::Unsupported(format!(
                "variadic array element type {other:?} requires explicit marshalling"
            )),
            unsupported => unsupported,
        };
    }
    let TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) = ty
    else {
        return classify_ts_type(ty, interfaces, generic_interfaces);
    };
    let mut payload = None;
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
            other => match classify_ts_type(other, interfaces, generic_interfaces) {
                DtsType::Native(ty) if payload.is_none() => payload = Some(ty),
                DtsType::Native(_) => {
                    return DtsType::Unsupported(
                        "variadic tagged union requires exactly one payload type".into(),
                    )
                }
                unsupported => return unsupported,
            },
        }
    }
    let Some(payload) = payload else {
        return DtsType::Unsupported("variadic tagged union has no payload type".into());
    };
    match (has_null, has_undefined) {
        (false, true) => DtsType::Native(HirType::Optional(Box::new(payload))),
        (true, false) => DtsType::Native(HirType::Nullable(Box::new(payload))),
        (true, true) => DtsType::Native(HirType::Nullish(Box::new(payload))),
        (false, false) => DtsType::Unsupported(
            "variadic union must include null or undefined in addition to its payload".into(),
        ),
    }
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
        HirType::Undefined => "undefined".to_string(),
        HirType::Null => "null".to_string(),
        HirType::Void => "void".to_string(),
        HirType::Str => "string".to_string(),
        HirType::Json => "Json".to_string(),
        HirType::Dictionary(element) => {
            format!("{{ [key: string]: {} }}", render_ts_type(element))
        }
        HirType::JsValue => "JsValue".to_string(),
        HirType::Array(elem) => format!("{}[]", render_ts_type(elem)),
        HirType::Tuple(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(render_ts_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        HirType::Promise(inner) => format!("Promise<{}>", render_ts_type(inner)),
        HirType::Optional(inner) => format!("{} | undefined", render_ts_type(inner)),
        HirType::Nullable(inner) => format!("{} | null", render_ts_type(inner)),
        HirType::Nullish(inner) => {
            format!("{} | null | undefined", render_ts_type(inner))
        }
        HirType::Object(fields) => {
            let rendered = fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", render_ts_type(ty)))
                .collect::<Vec<_>>()
                .join("; ");
            format!("{{ {rendered} }}")
        }
        HirType::Function(params, ret) => {
            let params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| format!("arg{index}: {}", render_ts_type(ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({params}) => {}", render_ts_type(ret))
        }
        HirType::CallableFunction(params, optional, rest, ret) => {
            let mut params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    format!(
                        "arg{index}{}: {}",
                        if optional.contains(index) { "?" } else { "" },
                        render_ts_type(ty)
                    )
                })
                .collect::<Vec<_>>();
            if let Some(rest) = rest {
                params.push(format!("...rest: {}[]", render_ts_type(rest)));
            }
            format!("({}) => {}", params.join(", "), render_ts_type(ret))
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
                let mut params = func
                    .params
                    .iter()
                    .zip(&sig.params)
                    .map(|((name, _), ty)| format!("{name}: {}", render_ts_type(ty)))
                    .collect::<Vec<_>>();
                if let (Some((name, _)), Some(variadic)) = (&func.rest_param, &sig.variadic) {
                    params.push(format!("...{name}: {}[]", render_ts_type(variadic)));
                }
                let params = params.join(", ");
                out.push_str(&format!(
                    "declare function {}({params}): {};\n",
                    sig.symbol,
                    render_ts_type(&sig.ret)
                ));
            }
            Classification::Fallback { function, reason } => {
                let returns_callable = functions
                    .iter()
                    .filter(|candidate| candidate.name == function)
                    .all(|candidate| {
                        matches!(
                            candidate.ret,
                            DtsType::Native(HirType::Function(_, _) | HirType::JsValue)
                        )
                    });
                let qualified_entry = qualified.iter().find(|q| q.name == function);
                if let Some(q) = qualified_entry {
                    out.push_str(&format!(
                        "// Fallback (QuickJS-NG), package-qualified alias: {reason}\n"
                    ));
                    if returns_callable {
                        out.push_str(&format!(
                            "function {}(argsArray: Json): JsValue {{\n",
                            q.alias
                        ));
                        out.push_str(&format!(
                            "    const callable: JsValue = getDynamicValue(\"{}\");\n    return callDynamicValueHandle(callable, argsArray);\n",
                            q.qualified_key
                        ));
                    } else {
                        out.push_str(&format!("function {}(argsArray: Json): Json {{\n", q.alias));
                        out.push_str(&format!(
                            "    return callDynamic(\"{}\", argsArray);\n",
                            q.qualified_key
                        ));
                    }
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
                if returns_callable {
                    out.push_str(&format!(
                        "function {function}(argsArray: Json): JsValue {{\n"
                    ));
                    out.push_str(&format!(
                        "    const callable: JsValue = getDynamicValue(\"{function}\");\n    return callDynamicValueHandle(callable, argsArray);\n"
                    ));
                } else {
                    out.push_str(&format!("function {function}(argsArray: Json): Json {{\n"));
                    out.push_str(&format!(
                        "    return callDynamic(\"{function}\", argsArray);\n"
                    ));
                }
                out.push_str("}\n");
            }
        }
    }
    out
}

/// Generates the same JSON-shaped fallback wrappers as `generate_shim`, but
/// targets the synchronous N-API host instead of QuickJS.
pub fn generate_native_addon_shim(
    functions: &[DtsFunction],
    qualified: &[QualifiedFallback],
) -> String {
    let mut out = String::new();
    for (function, _) in effective_classifications(functions, false) {
        if let Some(qualified) = qualified.iter().find(|entry| entry.name == function) {
            out.push_str("// Fallback (N-API), package-qualified alias\n");
            out.push_str(&format!(
                "function {}(argsArray: Json): Json {{\n",
                qualified.alias
            ));
            out.push_str(&format!(
                "    return callNativeAddon(\"{function}\", argsArray);\n"
            ));
            out.push_str("}\n");
            if qualified.suppress_bare {
                continue;
            }
        }
        out.push_str("// Fallback (N-API)\n");
        out.push_str(&format!("function {function}(argsArray: Json): Json {{\n"));
        out.push_str(&format!(
            "    return callNativeAddon(\"{function}\", argsArray);\n"
        ));
        out.push_str("}\n");
    }
    out
}

pub struct NativeAddon<'a> {
    pub package_name: &'a str,
    pub bytes: &'a [u8],
    pub root_export: Option<&'a str>,
}

pub fn generate_native_addon_init(addons: &[NativeAddon<'_>]) -> String {
    if addons.is_empty() {
        return String::new();
    }
    let mut out = String::from("function __thaw_native_module_init(): void {\n");
    for addon in addons {
        out.push_str(&format!("    // {}\n", addon.package_name));
        let hex: String = addon
            .bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let root_export = escape_ts_string_literal(addon.root_export.unwrap_or(""));
        out.push_str(&format!(
            "    loadNativeAddonEmbedded(\"{hex}\", \"{root_export}\");\n"
        ));
    }
    out.push_str("}\n");
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
         \x20\x20globalThis.process = {{ argv: [], env: {{}}, platform: 'linux', version: '', execPath: '/usr/bin/node', config: {{ variables: {{}} }}, versions: {{ node: '', modules: '', uv: '' }}, nextTick: function(fn) {{ var args = Array.prototype.slice.call(arguments, 1); var run = function() {{ fn.apply(undefined, args); }}; if (typeof queueMicrotask === 'function') queueMicrotask(run); else Promise.resolve().then(run); }} }};\n\
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
         var __thaw_bind_module_exports = function() {{\n\
         \x20\x20if (typeof module.exports === 'object' && module.exports !== null) {{ for (var k in module.exports) {{ globalThis[k] = module.exports[k]; }} }}\n\
         {bind_default_exports}\
         }};\n\
         if (globalThis.__thaw_module_ready && typeof globalThis.__thaw_module_ready.then === 'function') {{ globalThis.__thaw_module_ready.then(__thaw_bind_module_exports); }}\n\
         else {{ __thaw_bind_module_exports(); }}"
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
mod tests;
